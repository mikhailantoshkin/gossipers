use std::collections::HashSet;
use std::vec;
use std::{collections::HashMap, net::SocketAddrV4};

use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::{Receiver, Sender};
use tracing::{debug, info, instrument, warn, Level};

#[derive(Serialize, Deserialize, Debug)]
pub struct Message {
    pub src: SocketAddrV4,
    pub dst: SocketAddrV4,
    pub id: u32,
    pub reply_to: Option<u32>,
    pub payload: Payload,
}

#[derive(Debug)]
pub enum Event {
    Message(Message),
    Trigger(Trigger),
}

#[derive(Debug, Clone)]
pub enum Trigger {
    Register(SocketAddrV4),
    GossipRandom,
    GossipSuspects,
    Strike(SocketAddrV4),
}

#[derive(Serialize, Deserialize, Debug)]
pub enum Payload {
    Register,
    RegisterOk { known: Vec<SocketAddrV4> },
    GossipRandom { message: String },
    GossipRandomOk,
    GossipSuspect { suspects: HashSet<SocketAddrV4> },
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
    rx: Receiver<Event>,
    tx: Sender<Message>,
}

impl Node {
    pub fn new(addr: SocketAddrV4, rx: Receiver<Event>, tx: Sender<Message>) -> Self {
        Node {
            src: addr,
            cnt: 0,
            neighbours: HashMap::new(),
            rx,
            tx,
        }
    }

    pub async fn main_loop(&mut self) {
        while let Some(event) = self.rx.recv().await {
            for msg in self.step(event) {
                if self.tx.send(msg).await.is_err() {
                    info!("Publishing channel is closed, exiting");
                    return;
                };
            }
        }
    }

    #[instrument(level = Level::DEBUG, skip(self), ret(level = Level::DEBUG))]
    fn step(&mut self, event: Event) -> Vec<Message> {
        match event {
            Event::Trigger(trigger) => match trigger {
                Trigger::Register(dst) => {
                    self.neighbours.entry(dst).or_default();
                    vec![self.message(dst, None, Payload::Register)]
                }
                Trigger::GossipRandom => self.gossip(),
                Trigger::GossipSuspects => self.gossip_suspects(),
                Trigger::Strike(addr) => {
                    warn!("Received a strike for {}", addr);
                    self.neighbours.entry(addr).and_modify(|n| {
                        if n.strikes < 3 {
                            n.strikes += 1;
                        };
                    });
                    vec![]
                }
            },
            Event::Message(msg) => {
                self.neighbours.entry(msg.src).and_modify(|n| n.strikes = 0);
                match msg.payload {
                    Payload::Register => {
                        let neighbours: Vec<SocketAddrV4> =
                            self.neighbours.keys().cloned().collect();
                        self.neighbours.entry(msg.src).or_default();
                        vec![self.message(
                            msg.src,
                            Some(msg.id),
                            Payload::RegisterOk { known: neighbours },
                        )]
                    }
                    Payload::RegisterOk { known } => {
                        let to_register: Vec<SocketAddrV4> = known
                            .into_iter()
                            .filter(|n| !self.neighbours.contains_key(n))
                            .collect();
                        let mut messages: Vec<Message> = Vec::with_capacity(to_register.len());
                        for addr in to_register {
                            self.neighbours.insert(addr, Neighbour::default());
                            messages.push(self.message(
                                addr,
                                None,
                                Payload::Register,
                            ));
                        }
                        debug!("My neighbourhood is {:#?}", self.neighbours.keys());
                        messages
                    }
                    Payload::GossipRandom { message } => {
                        info!("Message from {}: {}", msg.src, message);
                        vec![self.message(msg.dst, Some(msg.id), Payload::GossipRandomOk)]
                    }
                    Payload::GossipSuspect { suspects } => {
                        info!(
                            "Recieved list of suspects from {}: {:#?}",
                            msg.src, suspects
                        );

                        let neighbourhood_size = self.neighbours.len();

                        for (a, n) in self.neighbours.iter_mut() {
                            if suspects.contains(a) {
                                n.suspected_by.insert(msg.src);
                            } else {
                                n.suspected_by.remove(&msg.src);
                            }
                            if neighbourhood_size >= 3
                                && n.suspected_by.len() > neighbourhood_size / 2
                            {
                                n.strikes = 3;
                                n.online = false;
                            }
                        }

                        vec![self.message(msg.src, Some(msg.id), Payload::GossipSuspectOk)]
                    }
                    Payload::GossipRandomOk | Payload::GossipSuspectOk => {
                        vec![]
                    }
                }
            }
        }
    }

    fn select_gossipers(&self) -> Vec<SocketAddrV4> {
        dbg!(&self.neighbours);
        self.neighbours
            .iter()
            .filter_map(|(a, n)| n.online.then_some(*a))
            .collect()
    }

    fn gossip_suspects(&mut self) -> Vec<Message> {
        let suspects: HashSet<_> = self
            .neighbours
            .iter()
            .filter(|(_, n)| n.strikes >= 3)
            .map(|(k, _)| *k)
            .collect();
        if suspects.is_empty() {
            info!("Not suspectins anyone of treason");
            return vec![];
        }
        let gossipers: Vec<_> = self.select_gossipers();
        info!(
            "Time to gossip suspects! Gossiping with {} neghbours",
            gossipers.len()
        );
        let mut messages = Vec::with_capacity(gossipers.len());
        for dst in gossipers {
            messages.push(self.message(
                dst,
                None,
                Payload::GossipSuspect {
                    suspects: suspects.clone(),
                },
            ));
        }
        messages
    }

    fn gossip(&mut self) -> Vec<Message> {
        let gossipers: Vec<_> = self.select_gossipers();
        info!(
            "Time to gossip! Gossiping with {} neghbours",
            gossipers.len()
        );
        let mut messages = Vec::with_capacity(gossipers.len());
        for dst in gossipers {
            messages.push(self.message(
                dst,
                None,
                Payload::GossipRandom {
                    message: format!("Some spicy scoop from {}", self.src),
                },
            ));
        }
        messages
    }

    fn message(&mut self, dst: SocketAddrV4, reply_to: Option<u32>, payload: Payload) -> Message {
        self.cnt += 1;
        Message {
            src: self.src,
            dst,
            id: self.cnt,
            reply_to,
            payload,
        }
    }
}
