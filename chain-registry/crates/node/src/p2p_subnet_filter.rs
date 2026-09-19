//! Anti-Sybil Subnet Ingress Bounding Filter
//!
//! Enforces IP subnet connection limits (/24 for IPv4, /48 for IPv6) to neutralize
//! Eclipse and Sybil attacks on GossipSub mesh networks, while preserving dedicated
//! priority ingress for bonded validators.

use libp2p::{multiaddr::Protocol, Multiaddr, PeerId};
use std::{
    collections::{HashMap, HashSet},
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
};

/// Default maximum concurrent active connections per /24 (IPv4) or /48 (IPv6) subnet
pub const DEFAULT_MAX_PEERS_PER_SUBNET: usize = 3;

/// Key representing a normalized network subnet
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SubnetKey {
    Ipv4Subnet([u8; 3]), // First 3 bytes of IPv4 (/24)
    Ipv6Subnet([u8; 6]), // First 6 bytes of IPv6 (/48)
    NonIp,               // Memory / loopback / relay
}

impl SubnetKey {
    pub fn from_ip(ip: IpAddr) -> Self {
        match ip {
            IpAddr::V4(v4) => {
                let octets = v4.octets();
                SubnetKey::Ipv4Subnet([octets[0], octets[1], octets[2]])
            }
            IpAddr::V6(v6) => {
                let segments = v6.octets();
                SubnetKey::Ipv6Subnet([
                    segments[0],
                    segments[1],
                    segments[2],
                    segments[3],
                    segments[4],
                    segments[5],
                ])
            }
        }
    }

    pub fn from_multiaddr(addr: &Multiaddr) -> Self {
        for proto in addr.iter() {
            match proto {
                Protocol::Ip4(v4) => return Self::from_ip(IpAddr::V4(v4)),
                Protocol::Ip6(v6) => return Self::from_ip(IpAddr::V6(v6)),
                _ => {}
            }
        }
        SubnetKey::NonIp
    }
}

/// Filter that restricts the number of active peers from the same IP subnet
#[derive(Debug)]
pub struct SubnetConnectionFilter {
    max_per_subnet: usize,
    active_connections: HashMap<SubnetKey, HashSet<PeerId>>,
    peer_to_subnet: HashMap<PeerId, SubnetKey>,
    bonded_validators: HashSet<PeerId>,
}

impl Default for SubnetConnectionFilter {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_PEERS_PER_SUBNET)
    }
}

impl SubnetConnectionFilter {
    pub fn new(max_per_subnet: usize) -> Self {
        Self {
            max_per_subnet: max_per_subnet.max(1),
            active_connections: HashMap::new(),
            peer_to_subnet: HashMap::new(),
            bonded_validators: HashSet::new(),
        }
    }

    /// Check if a connection from `addr` should be accepted for `peer_id`.
    /// Active bonded validators bypass subnet limits and do not count toward subnet caps.
    pub fn allow_and_record(
        &mut self,
        peer_id: &PeerId,
        addr: &Multiaddr,
        is_bonded_validator: bool,
    ) -> bool {
        let subnet = SubnetKey::from_multiaddr(addr);

        // If peer is already tracked in this subnet, permit continuation
        if let Some(existing_subnet) = self.peer_to_subnet.get(peer_id) {
            if *existing_subnet == subnet {
                return true;
            }
        }

        // Active bonded validators always bypass subnet limits
        if is_bonded_validator {
            self.bonded_validators.insert(*peer_id);
            self.insert_peer(*peer_id, subnet);
            return true;
        }

        // Loopback / Non-IP bypass
        if subnet == SubnetKey::NonIp {
            self.insert_peer(*peer_id, subnet);
            return true;
        }

        let regular_peer_count = self
            .active_connections
            .get(&subnet)
            .map(|set| {
                set.iter()
                    .filter(|p| !self.bonded_validators.contains(p))
                    .count()
            })
            .unwrap_or(0);

        if regular_peer_count >= self.max_per_subnet {
            tracing::warn!(
                "[AntiSybil] Rejected peer {} from subnet {:?} (cap {} reached)",
                peer_id,
                subnet,
                self.max_per_subnet
            );
            return false;
        }

        self.insert_peer(*peer_id, subnet);
        true
    }

    /// Record peer disconnection, freeing their subnet slot
    pub fn record_disconnect(&mut self, peer_id: &PeerId) {
        self.bonded_validators.remove(peer_id);
        if let Some(subnet) = self.peer_to_subnet.remove(peer_id) {
            if let Some(set) = self.active_connections.get_mut(&subnet) {
                set.remove(peer_id);
                if set.is_empty() {
                    self.active_connections.remove(&subnet);
                }
            }
        }
    }

    fn insert_peer(&mut self, peer_id: PeerId, subnet: SubnetKey) {
        self.peer_to_subnet.insert(peer_id, subnet.clone());
        self.active_connections
            .entry(subnet)
            .or_default()
            .insert(peer_id);
    }

    pub fn total_active_peers(&self) -> usize {
        self.peer_to_subnet.len()
    }

    pub fn distinct_subnets_count(&self) -> usize {
        self.active_connections.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_subnet_filter_limits_per_subnet() {
        let mut filter = SubnetConnectionFilter::new(2); // Max 2 per /24

        let addr1: Multiaddr = "/ip4/192.168.1.10/tcp/4001".parse().unwrap();
        let addr2: Multiaddr = "/ip4/192.168.1.55/tcp/4001".parse().unwrap();
        let addr3: Multiaddr = "/ip4/192.168.1.99/tcp/4001".parse().unwrap(); // 3rd in same subnet
        let diff_subnet: Multiaddr = "/ip4/10.0.0.1/tcp/4001".parse().unwrap();

        let p1 = PeerId::random();
        let p2 = PeerId::random();
        let p3 = PeerId::random();
        let p4 = PeerId::random();

        assert!(filter.allow_and_record(&p1, &addr1, false));
        assert!(filter.allow_and_record(&p2, &addr2, false));

        // 3rd peer from 192.168.1.0/24 should be rejected
        assert!(!filter.allow_and_record(&p3, &addr3, false));

        // Different subnet should be allowed
        assert!(filter.allow_and_record(&p4, &diff_subnet, false));

        // Bonded validator from 192.168.1.0/24 bypasses limit
        assert!(filter.allow_and_record(&p3, &addr3, true));

        // Disconnecting p1 frees up a slot
        filter.record_disconnect(&p1);
        let p5 = PeerId::random();
        assert!(filter.allow_and_record(&p5, &addr1, false));
    }
}
