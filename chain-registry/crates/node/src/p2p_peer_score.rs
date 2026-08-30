//! Dynamic P2P Peer Scoring & Reputation Matrix
//!
//! Tracks peer message delivery quality, signature verification outcomes,
//! rate limit violations, and enforces graylisting / blacklisting against toxic peers.

use libp2p::PeerId;
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

/// Default threshold below which a peer is graylisted (penalized/throttled)
pub const DEFAULT_GRAYLIST_THRESHOLD: f64 = 0.0;

/// Default threshold below which a peer is blacklisted (disconnected & banned)
pub const DEFAULT_BLACKLIST_THRESHOLD: f64 = -50.0;

/// Peer behavioral metrics
#[derive(Debug, Clone)]
pub struct PeerMetrics {
    pub valid_messages: u64,
    pub invalid_signatures: u64,
    pub corrupted_payloads: u64,
    pub rate_limit_violations: u64,
    pub score: f64,
    pub last_seen: Instant,
    pub banned_until: Option<Instant>,
}

impl Default for PeerMetrics {
    fn default() -> Self {
        Self {
            valid_messages: 0,
            invalid_signatures: 0,
            corrupted_payloads: 0,
            rate_limit_violations: 0,
            score: 10.0, // Initial neutral-positive bootstrap score
            last_seen: Instant::now(),
            banned_until: None,
        }
    }
}

/// Peer action state evaluated from score
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeerAction {
    /// Peer is healthy and permitted full gossip participation
    Allow,
    /// Peer is degraded; message propagation is throttled
    Graylist,
    /// Peer is toxic; drop messages, disconnect, and reject dials
    Blacklist,
}

/// Tracker for all connected and historical peer reputation scores
#[derive(Debug)]
pub struct PeerScoreTracker {
    scores: HashMap<PeerId, PeerMetrics>,
    graylist_threshold: f64,
    blacklist_threshold: f64,
    ban_duration: Duration,
}

impl Default for PeerScoreTracker {
    fn default() -> Self {
        Self::new(
            DEFAULT_GRAYLIST_THRESHOLD,
            DEFAULT_BLACKLIST_THRESHOLD,
            Duration::from_secs(300), // 5 minute default ban
        )
    }
}

impl PeerScoreTracker {
    pub fn new(graylist_threshold: f64, blacklist_threshold: f64, ban_duration: Duration) -> Self {
        Self {
            scores: HashMap::new(),
            graylist_threshold,
            blacklist_threshold,
            ban_duration,
        }
    }

    /// Record a valid message delivery (+1.0 reward, capped at 100.0)
    pub fn record_valid_message(&mut self, peer_id: &PeerId) {
        let entry = self.scores.entry(*peer_id).or_default();
        entry.valid_messages += 1;
        entry.score = (entry.score + 1.0).min(100.0);
        entry.last_seen = Instant::now();
    }

    /// Record an invalid signature delivery (-25.0 penalty)
    pub fn record_invalid_signature(&mut self, peer_id: &PeerId) -> PeerAction {
        let entry = self.scores.entry(*peer_id).or_default();
        entry.invalid_signatures += 1;
        entry.score -= 25.0;
        entry.last_seen = Instant::now();
        self.evaluate_action(peer_id)
    }

    /// Record a corrupted or malformed payload delivery (-15.0 penalty)
    pub fn record_corrupted_payload(&mut self, peer_id: &PeerId) -> PeerAction {
        let entry = self.scores.entry(*peer_id).or_default();
        entry.corrupted_payloads += 1;
        entry.score -= 15.0;
        entry.last_seen = Instant::now();
        self.evaluate_action(peer_id)
    }

    /// Record a rate limit breach (-10.0 penalty)
    pub fn record_rate_limit_violation(&mut self, peer_id: &PeerId) -> PeerAction {
        let entry = self.scores.entry(*peer_id).or_default();
        entry.rate_limit_violations += 1;
        entry.score -= 10.0;
        entry.last_seen = Instant::now();
        self.evaluate_action(peer_id)
    }

    /// Evaluate current peer action state
    pub fn evaluate_action(&mut self, peer_id: &PeerId) -> PeerAction {
        let now = Instant::now();
        let entry = self.scores.entry(*peer_id).or_default();

        // Check if existing ban is still active
        if let Some(banned_until) = entry.banned_until {
            if now < banned_until {
                return PeerAction::Blacklist;
            } else {
                // Ban expired: reset score to neutral baseline
                entry.banned_until = None;
                entry.score = 0.0;
            }
        }

        if entry.score < self.blacklist_threshold {
            entry.banned_until = Some(now + self.ban_duration);
            PeerAction::Blacklist
        } else if entry.score < self.graylist_threshold {
            PeerAction::Graylist
        } else {
            PeerAction::Allow
        }
    }

    /// Check if a peer is allowed without modifying state
    pub fn is_allowed(&self, peer_id: &PeerId) -> bool {
        if let Some(entry) = self.scores.get(peer_id) {
            if let Some(banned_until) = entry.banned_until {
                if Instant::now() < banned_until {
                    return false;
                }
            }
            entry.score >= self.graylist_threshold
        } else {
            true // Unknown peer starts with neutral allow
        }
    }

    /// Get current score for a peer
    pub fn get_score(&self, peer_id: &PeerId) -> f64 {
        self.scores
            .get(peer_id)
            .map(|m| m.score)
            .unwrap_or(10.0)
    }

    /// Clean up metrics for stale peers not seen in 1 hour
    pub fn prune_stale_peers(&mut self, max_age: Duration) {
        let now = Instant::now();
        self.scores
            .retain(|_, metrics| now.duration_since(metrics.last_seen) < max_age);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_peer_scoring_lifecycle() {
        let mut tracker = PeerScoreTracker::default();
        let peer = PeerId::random();

        assert_eq!(tracker.evaluate_action(&peer), PeerAction::Allow);
        assert_eq!(tracker.get_score(&peer), 10.0);

        // Deliver valid messages
        tracker.record_valid_message(&peer);
        tracker.record_valid_message(&peer);
        assert_eq!(tracker.get_score(&peer), 12.0);

        // Invalid signature drops score by 25
        let action = tracker.record_invalid_signature(&peer);
        assert_eq!(tracker.get_score(&peer), -13.0);
        assert_eq!(action, PeerAction::Graylist);

        // Another invalid signature drops score to -38 (still graylisted)
        let action2 = tracker.record_invalid_signature(&peer);
        assert_eq!(tracker.get_score(&peer), -38.0);
        assert_eq!(action2, PeerAction::Graylist);

        // Third invalid signature drops score to -63 (triggers blacklist/ban)
        let action3 = tracker.record_invalid_signature(&peer);
        assert_eq!(tracker.get_score(&peer), -63.0);
        assert_eq!(action3, PeerAction::Blacklist);
        assert!(!tracker.is_allowed(&peer));
    }
}
