mod cli;
mod telemetry;

use std::{collections::HashMap, net::SocketAddrV4, time::Duration};

use anyhow::Context;
use clap::Parser;
use cli::Cli;
use serde::{Deserialize, Serialize};
use telemetry::init_tracing;
use tokio::io::AsyncReadExt;
use tokio::sync::mpsc::{channel, error::SendError, Receiver, Sender};
use tokio::time::interval;
use tokio::{io::AsyncWriteExt, net::TcpListener};
use tokio_stream::wrappers::TcpListenerStream;
use tokio_stream::StreamExt;
use tracing::{debug, info, instrument};

#[derive(Serialize, Deserialize, Debug)]
struct Message {
    src: SocketAddrV4,
    dst: SocketAddrV4,
    id: u32,
    payload: Payload,
}

#[derive(Serialize, Deserialize, Debug)]
pub enum Payload {
    Register { addr: SocketAddrV4 },
    RegisterOk { known: Vec<SocketAddrV4> },
    GossipRandom { message: String },
    GossipRandomOk,
}

#[derive(Default, Debug)]
struct Neighbour {}

#[derive(Debug)]
struct Node {
    src: SocketAddrV4,
    cnt: u32,
    neighbours: HashMap<SocketAddrV4, Neighbour>,
    rx: Receiver<Message>,
    tx: Sender<Message>,
}

impl Node {
    fn new(addr: SocketAddrV4, rx: Receiver<Message>, tx: Sender<Message>) -> Self {
        Node {
            src: addr,
            cnt: 0,
            neighbours: HashMap::new(),
            rx,
            tx,
        }
    }
    async fn register(&mut self, dst: SocketAddrV4) -> Result<(), SendError<Message>> {
        let msg = Message {
            src: self.src,
            dst,
            id: self.cnt,
            payload: Payload::Register { addr: self.src },
        };
        self.cnt += 1;
        self.tx.send(msg).await
    }

    async fn main_loop(&mut self, gossip_interval: usize) {
        let mut interval = interval(Duration::from_secs(gossip_interval as u64));
        // first tick fires on the first call, call it once to actiualy wait for gossip_intreval
        interval.tick().await;
        loop {
            tokio::select! {
                maybe_v = self.rx.recv() => {
                    if let Some(msg) = maybe_v {
                        if self.handle_message(msg).await.is_err() {
                            // TODO gracefull exit?
                            break
                        }
                    } else {
                        break;
                    }
                }
                _ = interval.tick() => {
                    if self.gossip().await.is_err() {
                        break
                    };
                }
            }
        }
        info!("Finising?");
    }

    async fn handle_message(&mut self, msg: Message) -> anyhow::Result<()> {
        let msgs = self.step(msg);
        for msg in msgs {
            self.tx.send(msg).await?;
        }
        Ok(())
    }

    #[instrument(skip(self))]
    fn step(&mut self, msg: Message) -> Vec<Message> {
        let messages = match msg.payload {
            Payload::Register { addr } => {
                let neighbours: Vec<SocketAddrV4> = self.neighbours.keys().cloned().collect();
                self.neighbours.entry(addr).or_default();
                let msg = vec![Message {
                    src: msg.dst,
                    dst: msg.src,
                    id: self.cnt,
                    payload: Payload::RegisterOk { known: neighbours },
                }];
                self.cnt += 1;
                msg
            }
            Payload::RegisterOk { known } => {
                self.neighbours.entry(msg.src).or_default();
                let to_register: Vec<SocketAddrV4> = known
                    .into_iter()
                    .filter(|n| !self.neighbours.contains_key(n))
                    .collect();
                let mut messages: Vec<Message> = Vec::with_capacity(to_register.len());
                for addr in to_register {
                    self.neighbours.insert(addr, Neighbour::default());
                    messages.push(Message {
                        src: self.src,
                        dst: addr,
                        id: self.cnt,
                        payload: Payload::Register { addr: self.src },
                    });
                    self.cnt += 1;
                }
                messages
            }
            Payload::GossipRandom { message } => {
                info!("Message from {}: {}", msg.src, message);
                let msgs = vec![Message {
                    src: self.src,
                    dst: msg.dst,
                    id: self.cnt,
                    payload: Payload::GossipRandomOk,
                }];
                self.cnt += 1;
                msgs
            }
            Payload::GossipRandomOk => {
                vec![]
            }
        };
        messages
    }

    async fn gossip(&mut self) -> anyhow::Result<()> {
        let gossipers = self.neighbours.keys();
        info!(
            "Time to gossip! Gossiping with {} neghbours",
            gossipers.len()
        );
        for addr in self.neighbours.keys() {
            info!("Gossiping with {}", addr);
            self.tx
                .send(Message {
                    src: self.src,
                    dst: *addr,
                    id: self.cnt,
                    payload: Payload::GossipRandom {
                        message: format!("Some spicy scoop from {}", self.src),
                    },
                })
                .await?;
            self.cnt += 1;
        }
        Ok(())
    }
}

struct TcpSender {
    rx: Receiver<Message>,
    tx: Sender<Message>,
}

impl TcpSender {
    async fn run(&mut self) {
        while let Some(msg) = self.rx.recv().await {
            let mut stream = tokio::net::TcpStream::connect(msg.dst).await.unwrap();
            let data = serde_json::to_vec(&msg).unwrap();
            debug!("Sending message {:#?}", msg);
            stream.write_all(&data).await.unwrap();
        }
        info!("Channel closed, sender exiting");
    }
}
struct TcpReceiver {
    listener: TcpListenerStream,
    tx: Sender<Message>,
}

impl TcpReceiver {
    async fn new(src: SocketAddrV4, tx: Sender<Message>) -> std::io::Result<Self> {
        let listener = TcpListener::bind(src).await?;

        Ok(TcpReceiver {
            listener: TcpListenerStream::new(listener),
            tx,
        })
    }
    pub async fn run(&mut self) {
        while let Some(stream) = self.listener.next().await {
            let mut stream = stream.unwrap();
            let mut buf = Vec::new();
            stream.read_to_end(&mut buf).await.unwrap();
            self.tx
                .send(serde_json::from_slice(&buf).unwrap())
                .await
                .unwrap();
        }
    }
}
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();
    let args = Cli::parse();
    let addr = SocketAddrV4::new("127.0.0.1".parse().unwrap(), args.port);
    let (sender_tx, sender_rx) = channel(1000);
    let (node_tx, node_rx) = channel(1000);

    let mut receiver = TcpReceiver::new(addr, node_tx.clone()).await?;
    let _receiver_handle = tokio::spawn(async move { receiver.run().await });
    let _sender_handle = tokio::spawn(async {
        let mut sender = TcpSender {
            rx: sender_rx,
            tx: node_tx,
        };
        sender.run().await;
    });
    let mut node = Node::new(addr, node_rx, sender_tx);
    if let Some(registrar) = args.connect {
        node.register(registrar)
            .await
            .context("Unable to register")?;
    }
    node.main_loop(args.period).await;
    Ok(())
}
