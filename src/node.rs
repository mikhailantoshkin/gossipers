use std::collections::HashSet;
use std::vec;
use std::{collections::HashMap, net::SocketAddrV4, time::Duration};

use serde::{Deserialize, Serialize};

use tokio::sync::mpsc::{error::SendError, Receiver, Sender};
use tokio::time::interval;

use tracing::{debug, info, instrument, warn, Level};

#[derive(Serialize, Deserialize, Debug)]
pub struct Message {
    pub src: SocketAddrV4,
    pub dst: SocketAddrV4,
    pub id: u32,
    pub payload: Payload,
}

#[derive(Serialize, Deserialize, Debug)]
pub enum Payload {
    Register { addr: SocketAddrV4 },
    RegisterOk { known: Vec<SocketAddrV4> },
    GossipRandom { message: String },
    GossipRandomOk,
    Strike { addr: SocketAddrV4 },
    GossipSuspect { suspects: Vec<SocketAddrV4> },
    GossipSuspectOk,
}

#[derive(Debug)]
struct Neighbour {
    strikes: u8,
    suspected_by: HashSet<SocketAddrV4>,
    online: bool,
}

impl Default for Neighbour {
    fn default() -> Self {
        Neighbour {
            strikes: 0,
            suspected_by: HashSet::new(),
            online: true,
        }
    }
}

#[derive(Debug)]
pub struct Node {
    src: SocketAddrV4,
    cnt: u32,
    neighbours: HashMap<SocketAddrV4, Neighbour>,
    rx: Receiver<Message>,
    tx: Sender<Message>,
}

impl Node {
    pub fn new(addr: SocketAddrV4, rx: Receiver<Message>, tx: Sender<Message>) -> Self {
        Node {
            src: addr,
            cnt: 0,
            neighbours: HashMap::new(),
            rx,
            tx,
        }
    }
    pub async fn register(&mut self, dst: SocketAddrV4) -> Result<(), SendError<Message>> {
        let msg = Message {
            src: self.src,
            dst,
            id: self.cnt,
            payload: Payload::Register { addr: self.src },
        };
        self.cnt += 1;
        self.tx.send(msg).await
    }

    pub async fn main_loop(&mut self, gossip_interval: usize) {
        let mut gossip_interval = interval(Duration::from_secs(gossip_interval as u64));
        let mut gossip_suspects = interval(Duration::from_secs(1));
        // first tick fires on the first call, call it once to actiualy wait for gossip_intreval
        gossip_interval.tick().await;
        gossip_suspects.tick().await;
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
                _ = gossip_interval.tick() => {
                    if self.gossip().await.is_err() {
                        break
                    };
                }
             _ = gossip_suspects.tick() => {
                    if self.gossip_suspects().await.is_err() {
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

    #[instrument(level = Level::DEBUG, skip(self), ret(level = Level::DEBUG))]
    fn step(&mut self, msg: Message) -> Vec<Message> {
        self.neighbours.entry(msg.src).and_modify(|n| {
            n.online = true;
            n.suspected_by.remove(&self.src);
            n.strikes = 0
        });
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
                debug!("My neighbourhood is {:#?}", self.neighbours.keys());
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
            Payload::GossipSuspect { suspects } => {
                info!(
                    "Recieved list of suspects from {}: {:#?}",
                    msg.src, suspects
                );

                let nieghbours_len = self.neighbours.len();
                for suspect in suspects {
                    self.neighbours.entry(suspect).and_modify(|n| {
                        n.suspected_by.insert(msg.src);
                        if n.suspected_by.len() > nieghbours_len / 2 {
                            n.online = false;
                        }
                    });
                }
                let msgs = vec![Message {
                    src: self.src,
                    dst: msg.src,
                    id: self.cnt,
                    payload: Payload::GossipSuspectOk,
                }];
                self.cnt += 1;
                msgs
            }
            Payload::Strike { addr } => {
                warn!("Received a strike for {}", addr);
                let nieghbours_len = self.neighbours.len();
                self.neighbours.entry(addr).and_modify(|n| {
                    if n.strikes < 3 {
                        n.strikes += 1;
                    } else {
                        n.suspected_by.insert(self.src);
                        info!("{} is now suspected", addr);
                    }
                    if n.suspected_by.len() > nieghbours_len / 2 {
                        info!("{} is considered to be offline", addr);
                        n.online = false;
                    }
                });
                vec![]
            }
            Payload::GossipRandomOk | Payload::GossipSuspectOk => {
                vec![]
            }
        };
        messages
    }

    async fn gossip_suspects(&mut self) -> anyhow::Result<()> {
        let suspects: Vec<_> = self
            .neighbours
            .iter()
            .filter(|(_, n)| n.strikes >= 3 || !n.online)
            .map(|(k, _)| *k)
            .collect();
        if suspects.is_empty() {
            info!("Not suspectins anyone of treason");
            return Ok(());
        }
        let gossipers: Vec<_> = self.neighbours.iter().filter(|(_, n)| n.online).collect();
        info!(
            "Time to gossip suspects! Gossiping with {} neghbours",
            gossipers.len()
        );
        for (addr, _) in gossipers {
            info!("Gossiping with {}", addr);
            self.tx
                .send(Message {
                    src: self.src,
                    dst: *addr,
                    id: self.cnt,
                    payload: Payload::GossipSuspect {
                        suspects: suspects.clone(),
                    },
                })
                .await?;
            self.cnt += 1;
        }
        Ok(())
    }

    async fn gossip(&mut self) -> anyhow::Result<()> {
        let gossipers: Vec<_> = self.neighbours.iter().filter(|(_, n)| n.online).collect();
        info!(
            "Time to gossip! Gossiping with {} neghbours",
            gossipers.len()
        );
        for (addr, _) in gossipers {
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
