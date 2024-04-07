# Acknowledgment

This solution is inspired by fly.io [distributed systems challenge](https://fly.io/dist-sys/) and, more specifically, Jon Gjengset's [attempt at solving it](https://youtu.be/gboGyccRVXI?si=gV_dO6qyP3-2ej9Z).

# Assumptions

- The topology and, by extension, the network are considered to be flat
- There are no broadcast messages. The lack of broadcast messages and network partitions allows us to avoid the implementation of sophisticated neighbourhood selection algorithms. Thus being said, in the real world, this is rarely a viable tradeoff.
- There are no malicious actors in the network. In the real world it is paramount to use secure communication channels and have peer authentication mechanisms.
- All nodes in the system running the same version of the software. In the real world, backwards-compatibility is extremely important in designing the distributed system since it is almost guaranteed that at some point some subsets of nodes will be running different versions of the software.
- All message handling is infallible. Since no real work is being done through gossip, there is no failure state associated with it.

# Design decisions

The overall design relies on an actor model. There are four actor types: State Machine, Message Sender, Message Receiver and Ticker.

The main logic is encoded as a synchronous state machine. This state machine is being driven by a series of `Event`s. Since some of the logic 
can not be triggered through communication with other nodes, namely the gossip timeout, several "ticker" background tasks are present.
These background tasks drive the state machine through `Trigger`s. Communication with other nodes is encoded as `Message`s.

The communication protocol is separate from the state machine itself. Currently, the system uses JSON over TCP as an underlying communication protocol.
In the real world, often it's not a viable solution, so QUIC or event UDP might be a better alternative. As for serialization format, pretty much any binary format like Protobuf or CBOR is more desirable. 

Strictly speaking, the use of async rust in this implementation is not necessary. Every tokio task can be replaced with
a thread without any implication for core logic.

For simplicity, there is no persistent connection between peers. With a bigger amount of sent messages is it paramount,
at least for TCP, to keep a pool of open connections to amortize the cost of opening a new connection. 

The system implements a naive consensus algorithm for excluding "dead" nodes from gossip. Each node keeps track of its
neighbours. If a neighbour fails to participate in gossip 3 times in a row (either through failing to respond
or by being unavailable) it is considered suspicious. If half of the
nodes, excluding the current node, consider some node suspicious then this node is considered to be dead and is
excluded from further gossip rounds. The list of suspicious nodes itself is also gossiped.