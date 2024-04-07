use std::net::SocketAddrV4;
use std::time::Duration;

use clap::Parser;
use gossipers::cli::Cli;
use gossipers::node::{Event, Message, Node, Trigger};

use gossipers::telemetry::init_tracing;
use tokio::io::AsyncReadExt;
use tokio::sync::mpsc::{channel, Receiver, Sender};

use tokio::time::interval;
use tokio::{io::AsyncWriteExt, net::TcpListener};
use tokio_stream::wrappers::TcpListenerStream;
use tokio_stream::StreamExt;
use tracing::{debug, info, warn};

struct TcpSender {
    rx: Receiver<Message>,
    tx: Sender<Event>,
}

impl TcpSender {
    async fn run(&mut self) {
        while let Some(msg) = self.rx.recv().await {
            match tokio::net::TcpStream::connect(msg.dst).await {
                Ok(mut stream) => {
                    let data = serde_json::to_vec(&msg).unwrap();
                    debug!("Sending message {:#?}", msg);
                    stream.write_all(&data).await.unwrap();
                }
                Err(err) => {
                    warn!("Unable to connect to {}: {}", msg.dst, err);
                    if self
                        .tx
                        .send(Event::Trigger(Trigger::Strike(msg.dst)))
                        .await
                        .is_err()
                    {
                        info!("Channel closed, sender exiting");
                        return;
                    };
                }
            }
        }
    }
}
struct TcpReceiver {
    listener: TcpListenerStream,
    tx: Sender<Event>,
}

impl TcpReceiver {
    async fn new(src: SocketAddrV4, tx: Sender<Event>) -> std::io::Result<Self> {
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
                .send(Event::Message(serde_json::from_slice(&buf).unwrap()))
                .await
                .unwrap();
        }
    }
}

async fn ticker(tx: Sender<Event>, period: u64, trigger: Trigger) {
    let mut ticker = interval(Duration::from_secs(period));
    // first tick is always instantaneous
    ticker.tick().await;
    loop {
        ticker.tick().await;
        if tx.send(Event::Trigger(trigger.clone())).await.is_err() {
            info!("Channel closed, ticker exiting");
            return;
        };
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();
    let args = Cli::parse();
    let addr = SocketAddrV4::new("127.0.0.1".parse().unwrap(), args.port);
    let (sender_tx, sender_rx) = channel(1000);
    let (node_tx, node_rx) = channel(1000);
    if let Some(registrar) = args.connect {
        node_tx
            .send(Event::Trigger(Trigger::Register(registrar)))
            .await
            .expect("Sending cannot fail");
    }

    let mut receiver = TcpReceiver::new(addr, node_tx.clone()).await?;
    let mut sender = TcpSender {
        rx: sender_rx,
        tx: node_tx.clone(),
    };
    tokio::spawn(ticker(
        node_tx.clone(),
        args.period as u64,
        Trigger::GossipRandom,
    ));
    tokio::spawn(ticker(node_tx.clone(), 1, Trigger::GossipSuspects));
    tokio::spawn(ticker(node_tx.clone(), 10, Trigger::CheckReplies));
    tokio::spawn(async move { receiver.run().await });
    tokio::spawn(async move { sender.run().await });

    let mut node = Node::new(addr, node_rx, sender_tx);
    node.main_loop().await;
    Ok(())
}
