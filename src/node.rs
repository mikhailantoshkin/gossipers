use std::collections::HashSet;
use std::vec;
use std::{collections::HashMap, net::SocketAddrV4};

use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use tokio::sync::mpsc::{Receiver, Sender};
use tracing::{debug, info, instrument, warn, Level};

use crate::neighbourhood::Charge;
use crate::neighbourhood::Neighbourhood;

const STALE_TIMEOUT: Duration = Duration::from_secs(10);

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
    CheckReplies,
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

impl Payload {
    fn requires_reply(&self) -> bool {
        match self {
            Payload::Register
            | Payload::GossipRandom { message: _ }
            | Payload::GossipSuspect { suspects: _ } => true,
            Payload::RegisterOk { known: _ }
            | Payload::GossipRandomOk
            | Payload::GossipSuspectOk => false,
        }
    }
}

#[derive(Debug)]
pub struct Node {
    src: SocketAddrV4,
    cnt: u32,
    neighbourhood: Neighbourhood,
    rx: Receiver<Event>,
    tx: Sender<Message>,
    awaiting_reply: HashMap<u32, (SocketAddrV4, Instant)>,
}

impl Node {
    pub fn new(addr: SocketAddrV4, rx: Receiver<Event>, tx: Sender<Message>) -> Self {
        Node {
            src: addr,
            cnt: 0,
            neighbourhood: Neighbourhood::new(),
            rx,
            tx,
            awaiting_reply: HashMap::new(),
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
            Event::Trigger(trigger) => self.handle_trigger(trigger),
            Event::Message(msg) => self.handle_message(msg),
        }
    }

    fn handle_trigger(&mut self, trigger: Trigger) -> Vec<Message> {
        match trigger {
            Trigger::Register(dst) => {
                self.neighbourhood.register(dst);
                vec![self.message(dst, None, Payload::Register)]
            }
            Trigger::GossipRandom => self.gossip(),
            Trigger::GossipSuspects => self.gossip_suspects(),
            Trigger::Strike(addr) => {
                warn!("Received a strike for {}", addr);
                self.neighbourhood.accuse(addr, Charge::Connection);
                vec![]
            }
            Trigger::CheckReplies => {
                let stale_keys: Vec<_> = self
                    .awaiting_reply
                    .iter()
                    .filter_map(|(k, (_, instant))| {
                        instant.elapsed().gt(&STALE_TIMEOUT).then_some(k)
                    })
                    .cloned()
                    .collect();
                for stale in stale_keys {
                    let (dst, instant) = self.awaiting_reply.remove(&stale).unwrap();
                    warn!(
                        "Didn't receive reply in time from {} for message {}. Elapsed: {}",
                        dst,
                        stale,
                        instant.elapsed().as_secs()
                    );
                    self.neighbourhood.accuse(dst, Charge::Reply);
                }
                vec![]
            }
        }
    }

    fn handle_message(&mut self, msg: Message) -> Vec<Message> {
        self.neighbourhood.dismiss(msg.src, Charge::Connection);
        match msg.payload {
            Payload::Register => {
                let neighbours: Vec<SocketAddrV4> = self.neighbourhood.get_all_neighbours();
                self.neighbourhood.register(msg.src);
                vec![self.message(
                    msg.src,
                    Some(msg.id),
                    Payload::RegisterOk { known: neighbours },
                )]
            }
            Payload::RegisterOk { known } => {
                self.handle_reply(msg.reply_to, msg.src);
                let to_register: Vec<SocketAddrV4> = known
                    .into_iter()
                    .filter(|n| !self.neighbourhood.is_registered(n))
                    .collect();
                let mut messages: Vec<Message> = Vec::with_capacity(to_register.len());
                for addr in to_register {
                    self.neighbourhood.register(addr);
                    messages.push(self.message(addr, None, Payload::Register));
                }
                debug!("My neighbourhood is {:#?}", self.neighbourhood);
                messages
            }
            Payload::GossipRandom { message } => {
                info!("Message from {}: {}", msg.src, message);
                vec![self.message(msg.src, Some(msg.id), Payload::GossipRandomOk)]
            }
            Payload::GossipSuspect { suspects } => {
                debug!(
                    "Received list of suspects from {}: {:#?}",
                    msg.src, suspects
                );
                self.neighbourhood.report(suspects, msg.src);
                vec![self.message(msg.src, Some(msg.id), Payload::GossipSuspectOk)]
            }
            Payload::GossipRandomOk | Payload::GossipSuspectOk => {
                self.handle_reply(msg.reply_to, msg.src);
                vec![]
            }
        }
    }

    fn handle_reply(&mut self, reply_id: Option<u32>, src: SocketAddrV4) {
        match reply_id {
            Some(reply_id) => match self.awaiting_reply.remove(&reply_id) {
                Some((dst, sended_at)) => {
                    if dst != src {
                        warn!(
                            "Expected response for message_id {} from {}, but got response from {}",
                            reply_id, dst, src
                        );
                    } else {
                        self.neighbourhood.dismiss(src, Charge::Reply);
                        debug!(
                            "Got response for message_id {} from {}. Reply took {}",
                            reply_id,
                            src,
                            sended_at.elapsed().as_nanos()
                        );
                    }
                }
                None => {
                    warn!(
                        "Received reply for message_id {} from {}, but no reply is expected",
                        reply_id, src
                    );
                }
            },
            None => {
                warn!(
                    "Reply message from {} doesn't have the 'reply_to' field set",
                    src
                );
            }
        }
    }

    fn gossip_suspects(&mut self) -> Vec<Message> {
        let suspects: HashSet<_> = self.neighbourhood.get_suspects();
        if suspects.is_empty() {
            debug!("Not suspecting anyone of treason");
            return vec![];
        }
        let gossipers: Vec<_> = self.neighbourhood.select_gossipers();
        debug!(
            "Time to gossip suspects! Gossiping with {} neighbours",
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
        let gossipers: Vec<_> = self.neighbourhood.select_gossipers();
        info!(
            "Time to gossip! Gossiping with {} neighbours",
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
        if payload.requires_reply() {
            self.awaiting_reply.insert(self.cnt, (dst, Instant::now()));
        }
        Message {
            src: self.src,
            dst,
            id: self.cnt,
            reply_to,
            payload,
        }
    }
}
