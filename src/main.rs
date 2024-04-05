mod cli;
mod node;
mod telemetry;

use std::net::SocketAddrV4;

use anyhow::Context;
use clap::Parser;
use cli::Cli;
use node::{Message, Node};

use telemetry::init_tracing;
use tokio::io::AsyncReadExt;
use tokio::sync::mpsc::{channel, Receiver, Sender};

use tokio::{io::AsyncWriteExt, net::TcpListener};
use tokio_stream::wrappers::TcpListenerStream;
use tokio_stream::StreamExt;
use tracing::{debug, info, warn};

struct TcpSender {
    rx: Receiver<Message>,
    tx: Sender<Message>,
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
                    let res = self.tx
                        .send(Message {
                            src: msg.src,
                            dst: msg.src,
                            id: msg.id,
                            payload: node::Payload::Strike { addr: msg.dst },
                        })
                        .await;
                    if res.is_err() {
                        // channel is closed
                        break;
                    }
                }
            }
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
