```
Message {
	Register{addr: String},
	RegisterOk{known_nodes: Vec<String>},
	GossipRandom{data: String},
	GossipRandomOk,
}
```
On `Regiser`:
- Add node to gossip list
- Reply with `RegisterOk`, that includes the gossip list
On `RegisterOk`:
- Add a nodes to gossip list
- Send `Register` to nodes added to the gossip list
On `Gossip`:
- Reply `GossipOk`
- Print the message into console

Assumptions:
- Topology is flat.  This assumption is made because "the peer should send a random gossip message to _all_ the other peers", implying there is no neighborhood selection.
- Nodes are assumed to be added one at a time. In real world nodes can be added at any point in time and this will cause a race condition in current implementation. The better way of node discovery is through gossip itself
- Message handling is infallible. Since nodes do not perform any meaningful work there is no failure state for gossip besides the network inaccessibility.