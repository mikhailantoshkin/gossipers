# Gossipers

Small p2p gossiping app. For a design overview see [DESIGN.md](DESIGN.md)

## MSRV

Dictated by clap and is `v1.74.0`

## Installing

### Building from source
Update your rust toolchain with `rustup update` or by following instructions on [official website](https://www.rust-lang.org/learn/get-started)

Clone this repo and build it with `cargo build`

### Precompiled binary

## Running

Deploy the first node with 
```
cargo run -- --port 8080 --period 6
```
or by running the binary directly
```
./target/debug/gossipers --port 8080 --period 6`
```

Deploy a second node and connect it to the first one by passing its address to `--connect` argument 
```
cargo run -- --port 8081 --period 6 --connect '127.0.0.1:8080'
```

Now you should see the two nodes gossiping some spicy scoops every `--period` seconds!

## Logging

You can control the logging level through `RUST_LOG` environment variable like so
```
RUST_LOG=debug cargo run -- --port 8082 --period 6 --connect '127.0.0.1:8080'
```