// crates/node/src/p2p_rate_limit.rs
// Rate limiting for P2P gossip messages to prevent spam attacks.
//
// Limits per peer:
//   - Vote messages:        30 per minute (consensus votes)
//   - Block announcements:  10 per minute (block propagation)
//   - General messages:     100 per minute (other gossip)
//
// Violating peers are temporarily banned and their messages dropped.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use libp2p::PeerId;

/// Configuration for P2P rate limiting.
#[derive(Debug, Clone)]
pub struct P2PRateLimitConfig {
    /// Max vote messages per window.
    pub vote_limit: u32,
    /// Max block announcements per window.
    pub block_limit: u32,
    /// Max general messages per window.
    pub general_limit: u32,
    /// Sliding window duration.
    pub window: Duration,
    /// Ban duration for violating peers.
    pub ban_duration: Duration,
}

impl Default for P2PRateLimitConfig {
    fn default() -> Self {
        Self {
            vote_limit: 30,
            block_limit: 10,
            general_limit: 100,
            window: Duration::from_secs(60),
            ban_duration: Duration::from_secs(300), // 5 minute ban
        }
    }
}

/// Rate limit bucket for a peer.
#[derive(Debug)]
struct PeerBucket {
    /// Timestamps of recent messages within the window.
    vote_timestamps: Vec<Instant>,
    block_timestamps: Vec<Instant>,
    general_timestamps: Vec<Instant>,
    /// If banned, when the ban expires.
    banned_until: Option<Instant>,
    /// Total violations (for escalating penalties).
    violation_count: u32,
}

impl PeerBucket {
    fn new() -> Self {
        Self {
            vote_timestamps: Vec::new(),
            block_timestamps: Vec::new(),
            general_timestamps: Vec::new(),
            banned_until: None,
            violation_count: 0,
        }
    }

    /// Check if peer is currently banned.
    fn is_banned(&self) -> bool {
        self.banned_until.map_or(false, |until| Instant::now() < until)
    }

    /// Ban the peer for the specified duration.
    fn ban(&mut self, duration: Duration) {
        self.banned_until = Some(Instant::now() + duration);
        self.violation_count += 1;
        
        // Clear timestamps on ban
        self.vote_timestamps.clear();
        self.block_timestamps.clear();
        self.general_timestamps.clear();
    }

    /// Check and update timestamps for a specific message type.
    fn check_and_update(timestamps: &mut Vec<Instant>, window: Duration, limit: u32) -> bool {
        let now = Instant::now();
        let cutoff = now - window;
        
        // Remove old timestamps
        timestamps.retain(|&t| t > cutoff);
        
        // Check if under limit
        if timestamps.len() < limit as usize {
            timestamps.push(now);
            true
        } else {
            false
        }
    }

    /// Check vote message.
    fn check_vote(&mut self, window: Duration, limit: u32) -> bool {
        Self::check_and_update(&mut self.vote_timestamps, window, limit)
    }

    /// Check block announcement.
    fn check_block(&mut self, window: Duration, limit: u32) -> bool {
        Self::check_and_update(&mut self.block_timestamps, window, limit)
    }

    /// Check general message.
    fn check_general(&mut self, window: Duration, limit: u32) -> bool {
        Self::check_and_update(&mut self.general_timestamps, window, limit)
    }
}

/// P2P rate limiter state.
#[derive(Clone)]
pub struct P2PRateLimiter {
    peers: Arc<Mutex<HashMap<PeerId, PeerBucket>>>,
    config: P2PRateLimitConfig,
}

/// Lock a mutex, recovering gracefully if it was poisoned.
fn lock_peers<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            tracing::warn!("P2P rate limiter mutex poisoned; recovering");
            poisoned.into_inner()
        }
    }
}

impl P2PRateLimiter {
    pub fn new(config: P2PRateLimitConfig) -> Self {
        Self {
            peers: Arc::new(Mutex::new(HashMap::new())),
            config,
        }
    }

    /// Check if a vote message is allowed from this peer.
    /// Returns true if allowed, false if rate limited.
    pub fn check_vote(&self, peer: PeerId) -> bool {
        let mut peers = lock_peers(&self.peers);
        let bucket = peers.entry(peer).or_insert_with(PeerBucket::new);
        
        // If banned, reject immediately
        if bucket.is_banned() {
            return false;
        }
        
        let allowed = bucket.check_vote(self.config.window, self.config.vote_limit);
        
        if !allowed {
            // First violation - ban the peer
            let ban_duration = self.config.ban_duration * (bucket.violation_count + 1);
            bucket.ban(ban_duration);
            tracing::warn!(
                "P2P Rate limit: Peer {} exceeded vote limit (violation #{}) - banned for {:?}",
                peer, bucket.violation_count, ban_duration
            );
        }
        
        allowed
    }

    /// Check if a block announcement is allowed from this peer.
    /// Returns true if allowed, false if rate limited.
    pub fn check_block(&self, peer: PeerId) -> bool {
        let mut peers = lock_peers(&self.peers);
        let bucket = peers.entry(peer).or_insert_with(PeerBucket::new);
        
        // If banned, reject immediately
        if bucket.is_banned() {
            return false;
        }
        
        let allowed = bucket.check_block(self.config.window, self.config.block_limit);
        
        if !allowed {
            let ban_duration = self.config.ban_duration * (bucket.violation_count + 1);
            bucket.ban(ban_duration);
            tracing::warn!(
                "P2P Rate limit: Peer {} exceeded block limit (violation #{}) - banned for {:?}",
                peer, bucket.violation_count, ban_duration
            );
        }
        
        allowed
    }

    /// Check if a general message is allowed from this peer.
    /// Returns true if allowed, false if rate limited.
    pub fn check_general(&self, peer: PeerId) -> bool {
        let mut peers = lock_peers(&self.peers);
        let bucket = peers.entry(peer).or_insert_with(PeerBucket::new);
        
        // If banned, reject immediately
        if bucket.is_banned() {
            return false;
        }
        
        let allowed = bucket.check_general(self.config.window, self.config.general_limit);
        
        if !allowed {
            let ban_duration = self.config.ban_duration * (bucket.violation_count + 1);
            bucket.ban(ban_duration);
            tracing::warn!(
                "P2P Rate limit: Peer {} exceeded general limit (violation #{}) - banned for {:?}",
                peer, bucket.violation_count, ban_duration
            );
        }
        
        allowed
    }

    /// Check if a peer is currently banned.
    pub fn is_banned(&self, peer: PeerId) -> bool {
        let peers = lock_peers(&self.peers);
        peers.get(&peer).map_or(false, |b| b.is_banned())
    }

    /// Get ban info for a peer.
    pub fn get_ban_info(&self, peer: PeerId) -> Option<(bool, u32, Option<Instant>)> {
        let peers = lock_peers(&self.peers);
        peers.get(&peer).map(|b| {
            (b.is_banned(), b.violation_count, b.banned_until)
        })
    }

    /// Manually ban a peer (for governance actions).
    pub fn ban_peer(&self, peer: PeerId, duration: Duration) {
        let mut peers = lock_peers(&self.peers);
        let bucket = peers.entry(peer).or_insert_with(PeerBucket::new);
        bucket.ban(duration);
        tracing::info!("P2P Rate limit: Peer {} manually banned for {:?}", peer, duration);
    }

    /// Unban a peer.
    pub fn unban_peer(&self, peer: PeerId) {
        let mut peers = lock_peers(&self.peers);
        if let Some(bucket) = peers.get_mut(&peer) {
            bucket.banned_until = None;
            tracing::info!("P2P Rate limit: Peer {} unbanned", peer);
        }
    }

    /// Purge expired entries from the rate limiter.
    /// Call periodically to prevent memory growth.
    pub fn purge_expired(&self) {
        let now = Instant::now();
        let window = self.config.window;
        
        let mut peers = lock_peers(&self.peers);
        peers.retain(|peer_id, bucket| {
            // Remove old timestamps
            bucket.vote_timestamps.retain(|&t| now - t < window);
            bucket.block_timestamps.retain(|&t| now - t < window);
            bucket.general_timestamps.retain(|&t| now - t < window);
            
            // Keep if peer has recent activity or is banned
            let has_activity = !bucket.vote_timestamps.is_empty()
                || !bucket.block_timestamps.is_empty()
                || !bucket.general_timestamps.is_empty();
            let is_banned = bucket.is_banned();
            
            if !has_activity && !is_banned {
                tracing::debug!("P2P Rate limit: Purging inactive peer {}", peer_id);
            }
            
            has_activity || is_banned
        });
    }

    /// Get stats about the rate limiter.
    pub fn get_stats(&self) -> P2PRateLimitStats {
        let peers = lock_peers(&self.peers);
        let total_peers = peers.len();
        let banned_peers = peers.values().filter(|b| b.is_banned()).count();
        let total_violations: u32 = peers.values().map(|b| b.violation_count).sum();
        
        P2PRateLimitStats {
            total_peers,
            banned_peers,
            total_violations,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct P2PRateLimitStats {
    pub total_peers: usize,
    pub banned_peers: usize,
    pub total_violations: u32,
}

/// Spawn a background task that purges expired rate limit entries.
pub fn spawn_purge_task(limiter: P2PRateLimiter) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        loop {
            interval.tick().await;
            limiter.purge_expired();
            let stats = limiter.get_stats();
            tracing::debug!(
                "P2P Rate limiter purged: {} peers, {} banned, {} total violations",
                stats.total_peers, stats.banned_peers, stats.total_violations
            );
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_peer() -> PeerId {
        PeerId::random()
    }

    #[test]
    fn allows_messages_within_limit() {
        let limiter = P2PRateLimiter::new(P2PRateLimitConfig {
            vote_limit: 5,
            ..Default::default()
        });
        let peer = test_peer();

        for _ in 0..5 {
            assert!(limiter.check_vote(peer));
        }
    }

    #[test]
    fn blocks_messages_over_limit() {
        let limiter = P2PRateLimiter::new(P2PRateLimitConfig {
            vote_limit: 3,
            ban_duration: Duration::from_secs(1), // Short ban for testing
            ..Default::default()
        });
        let peer = test_peer();

        // Use up the limit
        for _ in 0..3 {
            assert!(limiter.check_vote(peer));
        }

        // Next message should be blocked and peer banned
        assert!(!limiter.check_vote(peer));
        assert!(limiter.is_banned(peer));
    }

    #[test]
    fn different_peers_have_independent_limits() {
        let limiter = P2PRateLimiter::new(P2PRateLimitConfig {
            vote_limit: 1,
            ..Default::default()
        });
        let peer1 = test_peer();
        let peer2 = test_peer();

        assert!(limiter.check_vote(peer1));
        assert!(!limiter.check_vote(peer1)); // peer1 exhausted
        assert!(limiter.check_vote(peer2));  // peer2 still fresh
    }

    #[test]
    fn different_message_types_are_independent() {
        let limiter = P2PRateLimiter::new(P2PRateLimitConfig {
            vote_limit: 1,
            block_limit: 1,
            general_limit: 100,
            ..Default::default()
        });
        let peer = test_peer();

        assert!(limiter.check_vote(peer));
        assert!(!limiter.check_vote(peer));   // vote exhausted
        assert!(limiter.check_block(peer));   // block still available
        assert!(!limiter.check_block(peer));  // block exhausted
        assert!(limiter.check_general(peer)); // general still available
    }

    #[test]
    fn manual_ban_works() {
        let limiter = P2PRateLimiter::new(P2PRateLimitConfig::default());
        let peer = test_peer();

        assert!(!limiter.is_banned(peer));
        
        limiter.ban_peer(peer, Duration::from_secs(60));
        assert!(limiter.is_banned(peer));
        
        limiter.unban_peer(peer);
        assert!(!limiter.is_banned(peer));
    }

    #[test]
    fn banned_peer_messages_rejected() {
        let limiter = P2PRateLimiter::new(P2PRateLimitConfig::default());
        let peer = test_peer();

        limiter.ban_peer(peer, Duration::from_secs(60));
        
        // All message types should be rejected while banned
        assert!(!limiter.check_vote(peer));
        assert!(!limiter.check_block(peer));
        assert!(!limiter.check_general(peer));
    }
}
