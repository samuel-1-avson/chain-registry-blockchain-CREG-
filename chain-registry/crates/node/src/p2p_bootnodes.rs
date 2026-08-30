//! Resilient Bootnode Discovery Manager
//!
//! Manages bootstrap node multiaddrs, tracks connection health with exponential
//! backoff, and ensures persistent Kademlia DHT routing table connectivity.

use libp2p::{Multiaddr, PeerId};
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

/// Health status of a bootstrap node
#[derive(Debug, Clone)]
pub struct BootnodeHealth {
    pub multiaddr: Multiaddr,
    pub peer_id: Option<PeerId>,
    pub consecutive_failures: u32,
    pub last_dial_attempt: Option<Instant>,
    pub last_successful_connected: Option<Instant>,
    pub is_connected: bool,
}

impl BootnodeHealth {
    pub fn new(multiaddr: Multiaddr, peer_id: Option<PeerId>) -> Self {
        Self {
            multiaddr,
            peer_id,
            consecutive_failures: 0,
            last_dial_attempt: None,
            last_successful_connected: None,
            is_connected: false,
        }
    }

    /// Compute backoff delay before the next dial is eligible
    pub fn backoff_duration(&self) -> Duration {
        let base_secs = 2u64.saturating_pow(self.consecutive_failures.min(6));
        Duration::from_secs(base_secs.clamp(1, 60))
    }

    /// Check if this bootnode is eligible for a redial attempt
    pub fn is_ready_for_dial(&self, now: Instant) -> bool {
        if self.is_connected {
            return false;
        }
        match self.last_dial_attempt {
            Some(last) => now.duration_since(last) >= self.backoff_duration(),
            None => true,
        }
    }
}

/// Manager coordinating bootnodes and DHT discovery
#[derive(Debug)]
pub struct BootnodeManager {
    bootnodes: Vec<BootnodeHealth>,
    min_desired_peers: usize,
    last_dht_refresh: Instant,
    dht_refresh_interval: Duration,
}

impl BootnodeManager {
    pub fn new(seed_addrs: Vec<Multiaddr>, min_desired_peers: usize) -> Self {
        let bootnodes = seed_addrs
            .into_iter()
            .map(|addr| {
                let peer_id = addr.iter().find_map(|p| match p {
                    libp2p::multiaddr::Protocol::P2p(id) => Some(id),
                    _ => None,
                });
                BootnodeHealth::new(addr, peer_id)
            })
            .collect();

        Self {
            bootnodes,
            min_desired_peers: min_desired_peers.max(4),
            last_dht_refresh: Instant::now(),
            dht_refresh_interval: Duration::from_secs(60),
        }
    }

    /// Retrieve all bootnodes ready to be dialed
    pub fn get_dials_due(&mut self, now: Instant) -> Vec<Multiaddr> {
        let mut to_dial = Vec::new();
        for node in &mut self.bootnodes {
            if node.is_ready_for_dial(now) {
                node.last_dial_attempt = Some(now);
                to_dial.push(node.multiaddr.clone());
            }
        }
        to_dial
    }

    /// Mark a bootnode as successfully connected
    pub fn mark_connected(&mut self, addr: &Multiaddr) {
        for node in &mut self.bootnodes {
            if &node.multiaddr == addr {
                node.is_connected = true;
                node.consecutive_failures = 0;
                node.last_successful_connected = Some(Instant::now());
                break;
            }
        }
    }

    /// Mark a bootnode dial as failed
    pub fn mark_failed(&mut self, addr: &Multiaddr) {
        for node in &mut self.bootnodes {
            if &node.multiaddr == addr {
                node.is_connected = false;
                node.consecutive_failures = node.consecutive_failures.saturating_add(1);
                break;
            }
        }
    }

    /// Mark a bootnode as disconnected
    pub fn mark_disconnected(&mut self, addr: &Multiaddr) {
        for node in &mut self.bootnodes {
            if &node.multiaddr == addr {
                node.is_connected = false;
                break;
            }
        }
    }

    /// Check if Kademlia DHT bootstrap or routing refresh is recommended
    pub fn needs_dht_refresh(&mut self, current_peer_count: usize, now: Instant) -> bool {
        if current_peer_count < self.min_desired_peers {
            if now.duration_since(self.last_dht_refresh) >= Duration::from_secs(15) {
                self.last_dht_refresh = now;
                return true;
            }
        } else if now.duration_since(self.last_dht_refresh) >= self.dht_refresh_interval {
            self.last_dht_refresh = now;
            return true;
        }
        false
    }

    pub fn total_bootnodes(&self) -> usize {
        self.bootnodes.len()
    }

    pub fn connected_bootnodes_count(&self) -> usize {
        self.bootnodes.iter().filter(|n| n.is_connected).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bootnode_manager_dial_and_backoff() {
        let addr: Multiaddr = "/ip4/127.0.0.1/tcp/4001".parse().unwrap();
        let mut manager = BootnodeManager::new(vec![addr.clone()], 5);

        let now = Instant::now();
        let dials = manager.get_dials_due(now);
        assert_eq!(dials.len(), 1);

        // Immediate next check should not return dial due to backoff
        let dials_empty = manager.get_dials_due(now);
        assert!(dials_empty.is_empty());

        // Mark failed increments failures and backoff
        manager.mark_failed(&addr);
        assert_eq!(manager.bootnodes[0].consecutive_failures, 1);

        // Mark connected resets failures
        manager.mark_connected(&addr);
        assert_eq!(manager.bootnodes[0].consecutive_failures, 0);
        assert_eq!(manager.connected_bootnodes_count(), 1);
    }
}
