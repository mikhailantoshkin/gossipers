mod cli;
use std::{
    collections::HashMap,
    net::{SocketAddrV4, TcpListener, TcpStream},
};

use clap::Parser;
use cli::Cli;
use serde::{Deserialize, Serialize};

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
}

impl Node {
    fn new(addr: SocketAddrV4) -> Self {
        Node {
            src: addr,
            cnt: 0,
            neighbours: HashMap::new(),
        }
    }
    fn register(&mut self, dst: SocketAddrV4) -> Message {
        self.cnt += 1;
        Message {
            src: self.src,
            dst,
            id: self.cnt,
            payload: Payload::Register { addr: self.src },
        }
    }

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
                dbg!(&self);
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

                dbg!(&self);
                messages
            }
            Payload::GossipRandom { message } => {
                println!("Message from {}: {}", msg.src, message);
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
}
fn main() {
    let args = Cli::parse();
    let addr = SocketAddrV4::new("127.0.0.1".parse().unwrap(), args.port);
    let listener = TcpListener::bind(addr).unwrap();
    let mut node = Node::new(addr);
    if let Some(dst) = args.connect {
        let msg = node.register(dst);
        let stream = TcpStream::connect(msg.dst).unwrap();
        serde_json::to_writer(&stream, &msg).unwrap();
    }
    for stream in listener.incoming() {
        let stream = stream.unwrap();
        let msg: Message = serde_json::from_reader(stream).unwrap();
        println!("Message: {:#?}", &msg);
        let responses = node.step(msg);
        for msg in responses {
            let stream = TcpStream::connect(msg.dst).unwrap();
            serde_json::to_writer(&stream, &msg).unwrap();
        }
    }
}
