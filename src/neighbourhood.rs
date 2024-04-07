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
struct Neighbour {
    suspicion: Suspicion,
    suspected_by: HashSet<SocketAddrV4>,
    online: bool,
}

impl Default for Neighbour {
    fn default() -> Self {
        Neighbour {
            suspicion: Suspicion::default(),
            suspected_by: HashSet::new(),
            online: true,
        }
    }
}

#[derive(Debug)]
pub struct Neighbourhood(HashMap<SocketAddrV4, Neighbour>);

impl Default for Neighbourhood {
    fn default() -> Self {
        Self::new()
    }
}

impl Neighbourhood {
    pub fn new() -> Self {
        Self(HashMap::new())
    }

    /// Add a new neighbour evicting existing neighbour with the same address
    pub fn register(&mut self, neighbour: SocketAddrV4) {
        self.0.insert(neighbour, Neighbour::default());
    }

    /// Accuse a neighbour of `Charge`. If neighbour is accused of enough charges, they
    /// are considered suspicious.
    pub fn accuse(&mut self, neighbour: SocketAddrV4, charge: Charge) {
        self.0
            .entry(neighbour)
            .and_modify(|n| n.suspicion.accuse(charge));
    }

    /// Releases all `Charge`s made against said neighbour.
    pub fn dismiss(&mut self, neighbour: SocketAddrV4, charge: Charge) {
        self.0
            .entry(neighbour)
            .and_modify(|n| n.suspicion.object(charge));
    }

    /// Report several neighbours of being suspicious. If a neighbour is accused by more then half of the
    /// other neighbours it is excluded form gossiping
    pub fn report(&mut self, suspects: HashSet<SocketAddrV4>, accuser: SocketAddrV4) {
        let neighbourhood_size = self.0.len();
        for (a, n) in self.0.iter_mut() {
            if suspects.contains(a) {
                n.suspected_by.insert(accuser);
            } else {
                n.suspected_by.remove(&accuser);
            }
            if neighbourhood_size >= 3 && n.suspected_by.len() > neighbourhood_size / 2 {
                n.suspicion.jury_ruling();
                n.online = false;
            } else {
                n.online = true;
            }
        }
    }

    /// Get all neighbours considered suspicious by current node. The neighbour is suspicious when they accused of enough
    /// `Charge`s of the same type. Being suspicious does not exclude them from gossiping
    pub fn get_suspects(&self) -> HashSet<SocketAddrV4> {
        self.0
            .iter()
            .filter_map(|(a, n)| n.suspicion.is_suspicious().then_some(*a))
            .collect()
    }

    /// Select neighbours to gossip with. Exclude neighbours that are considered suspicious by majority of the neighbourhood
    pub fn select_gossipers(&self) -> Vec<SocketAddrV4> {
        self.0
            .iter()
            .filter_map(|(a, n)| n.online.then_some(*a))
            .collect()
    }

    pub fn get_all_neighbours(&self) -> Vec<SocketAddrV4> {
        self.0.keys().cloned().collect()
    }

    pub fn is_registered(&self, neighbour: &SocketAddrV4) -> bool {
        self.0.contains_key(neighbour)
    }
}
