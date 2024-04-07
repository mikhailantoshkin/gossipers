use std::{
    collections::{HashMap, HashSet},
    net::SocketAddrV4,
};

pub enum Charge {
    Connection,
    Reply,
}

#[derive(Debug, Default)]
struct Suspicion {
    connection: u8,
    reply: u8,
}

impl Suspicion {
    fn accuse(&mut self, charge: Charge) {
        match charge {
            Charge::Connection => self.connection += 1,
            Charge::Reply => self.reply += 1,
        };
    }

    fn object(&mut self, charge: Charge) {
        match charge {
            Charge::Connection => self.connection = 0,
            Charge::Reply => self.reply = 0,
        };
    }

    fn jury_ruling(&mut self) {
        self.connection = 3;
        self.reply = 3;
    }

    fn is_suspicious(&self) -> bool {
        self.reply >= 3 || self.connection >= 3
    }
}

#[derive(Debug)]
struct Neighbor {
    suspicion: Suspicion,
    suspected_by: HashSet<SocketAddrV4>,
    online: bool,
}

impl Default for Neighbor {
    fn default() -> Self {
        Neighbor {
            suspicion: Suspicion::default(),
            suspected_by: HashSet::new(),
            online: true,
        }
    }
}

#[derive(Debug)]
pub struct Neighborhood(HashMap<SocketAddrV4, Neighbor>);

impl Default for Neighborhood {
    fn default() -> Self {
        Self::new()
    }
}

impl Neighborhood {
    pub fn new() -> Self {
        Self(HashMap::new())
    }

    /// Add a new neighbor evicting existing neighbor with the same address
    pub fn register(&mut self, neighbor: SocketAddrV4) {
        self.0.insert(neighbor, Neighbor::default());
    }

    /// Accuse a neighbor of `Charge`. If neighbor is accused of enough charges, they
    /// are considered suspicious.
    pub fn accuse(&mut self, neighbor: SocketAddrV4, charge: Charge) {
        self.0
            .entry(neighbor)
            .and_modify(|n| n.suspicion.accuse(charge));
    }

    /// Releases all `Charge`s made against said neighbor.
    pub fn dismiss(&mut self, neighbor: SocketAddrV4, charge: Charge) {
        self.0
            .entry(neighbor)
            .and_modify(|n| n.suspicion.object(charge));
    }

    /// Report several neighbors of being suspicious. If a neighbor is accused by more then half of the
    /// other neighbors it is excluded form gossiping
    pub fn report(&mut self, suspects: HashSet<SocketAddrV4>, accuser: SocketAddrV4) {
        let neighborhood_size = self.0.len();
        for (a, n) in self.0.iter_mut() {
            if suspects.contains(a) {
                n.suspected_by.insert(accuser);
            } else {
                n.suspected_by.remove(&accuser);
            }
            if neighborhood_size >= 3 && n.suspected_by.len() > neighborhood_size / 2 {
                n.suspicion.jury_ruling();
                n.online = false;
            } else {
                n.online = true;
            }
        }
    }

    /// Get all neighbors considered suspicious by current node. The neighbor is suspicious when they accused of enough
    /// `Charge`s of the same type. Being suspicious does not exclude them from gossiping
    pub fn get_suspects(&self) -> HashSet<SocketAddrV4> {
        self.0
            .iter()
            .filter_map(|(a, n)| n.suspicion.is_suspicious().then_some(*a))
            .collect()
    }

    /// Select neighbors to gossip with. Exclude neighbors that are considered suspicious by majority of the neighborhood
    pub fn select_gossipers(&self) -> Vec<SocketAddrV4> {
        self.0
            .iter()
            .filter_map(|(a, n)| n.online.then_some(*a))
            .collect()
    }

    pub fn get_all_neighbors(&self) -> Vec<SocketAddrV4> {
        self.0.keys().cloned().collect()
    }

    pub fn is_registered(&self, neighbor: &SocketAddrV4) -> bool {
        self.0.contains_key(neighbor)
    }
}
