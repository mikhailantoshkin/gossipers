# Gossipers

Small p2p gossiping app. For a design overview see [DESIGN.md](DESIGN.md)

## MSRV

Dictated by clap and is `v1.74.0`

## Installing

### Building from source
Update your rust toolchain with `rustup update` or by following instructions on [official website](https://www.rust-lang.org/learn/get-started)

Clone this repo and install it with `cargo install --path .`

### Precompiled binary

Grab a binary for your platform from [Releases](https://github.com/mikhailantoshkin/gossipers/releases) page 

## Running

Deploy the first node with 
```
gossipers --port 8080 --period 6
```

Deploy a second node and connect it to the first one by passing its address to `--connect` argument 
```
gossipers --port 8081 --period 6 --connect '127.0.0.1:8080'
```

Now you should see the two nodes gossiping some spicy scoops every `--period` seconds!

## Logging

You can control the logging level through `RUST_LOG` environment variable like so
```
RUST_LOG=debug gossipers --port 8082 --period 6 --connect '127.0.0.1:8080'
```