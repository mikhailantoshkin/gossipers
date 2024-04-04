use std::net::SocketAddrV4;

use clap::Parser;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct Cli {
    /// Port to listen on
    #[arg(short, long)]
    pub port: u16,

    /// Period to send gossip messages, seconds
    #[arg(long)]
    pub period: usize,

    /// Address of the node to connect to
    #[arg(long)]
    pub connect: Option<SocketAddrV4>,
}
