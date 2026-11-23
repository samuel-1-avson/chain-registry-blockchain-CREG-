//! Stateless proof-of-work challenges (ADR-019 §5.4, `ECA-31`).
//!
//! A challenge is `v1.<issued_unix>.<random>.<difficulty>.<tag>` where
//! `tag = HMAC-SHA256(key, "creg-faucet-pow-v1|issued|random|difficulty|ip_bucket")`.
//! The server keeps no per-challenge state: the tag binds the issue time, the
//! difficulty and the requester's IP bucket, so a challenge cannot be
//! downgraded, extended or replayed from another network. Single use is
//! enforced durably by recording the redeemed challenge id in PostgreSQL.
//!
//! Issuance is bounded: a per-IP rate (fixed one-minute windows, a hard cap
//! on tracked IPs) and an adaptive difficulty that rises with the global
//! issuance rate. Clients keep treating the challenge as an opaque string:
//! they search for a nonce with `SHA-256(challenge || nonce)` having
//! `difficulty` leading zero bits.

use hmac::{Hmac, Mac};
use rand::RngCore;
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use zeroize::Zeroizing;

type HmacSha256 = Hmac<Sha256>;

const VERSION: &str = "v1";
const DOMAIN: &str = "creg-faucet-pow-v1";
/// Upper bound on the adaptive difficulty.
pub const MAX_DIFFICULTY: u8 = 26;
/// Upper bound on distinct IP buckets tracked per window (`ECO-INV-08`).
pub const MAX_TRACKED_IPS: usize = 20_000;
const WINDOW: Duration = Duration::from_secs(60);

#[derive(Debug, PartialEq, Eq)]
pub enum PowError {
    Malformed,
    BadTag,
    Expired,
    FromTheFuture,
    InsufficientWork,
}

impl PowError {
    pub fn metric_reason(&self) -> &'static str {
        match self {
            Self::Malformed => "malformed",
            Self::BadTag => "bad_tag",
            Self::Expired => "expired",
            Self::FromTheFuture => "future",
            Self::InsufficientWork => "invalid_solution",
        }
    }
}

/// A verified challenge: its stable id for single-use recording.
#[derive(Debug, PartialEq, Eq)]
pub struct VerifiedChallenge {
    pub challenge_id: String,
}

pub struct PowIssuer {
    key: Zeroizing<[u8; 32]>,
    ttl: Duration,
    base_difficulty: u8,
    per_ip_per_minute: u32,
    window: Mutex<IssueWindow>,
}

#[derive(Default)]
struct IssueWindow {
    started_unix: u64,
    total: u32,
    per_ip: HashMap<String, u32>,
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

impl PowIssuer {
    /// `key` must be 32 random bytes (a random per-process key is fine for a
    /// single faucet instance: a restart only invalidates open challenges).
    pub fn new(key: [u8; 32], ttl: Duration, base_difficulty: u8, per_ip_per_minute: u32) -> Self {
        Self {
            key: Zeroizing::new(key),
            ttl,
            base_difficulty: base_difficulty.clamp(8, MAX_DIFFICULTY),
            per_ip_per_minute: per_ip_per_minute.max(1),
            window: Mutex::new(IssueWindow::default()),
        }
    }

    pub fn random_key() -> [u8; 32] {
        let mut key = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut key);
        key
    }

    pub fn ttl(&self) -> Duration {
        self.ttl
    }

    fn tag(&self, issued: u64, random: &str, difficulty: u8, ip_bucket: &str) -> String {
        let mut mac = match HmacSha256::new_from_slice(self.key.as_slice()) {
            Ok(mac) => mac,
            // HMAC accepts keys of any length; this branch is unreachable.
            Err(_) => return String::new(),
        };
        mac.update(format!("{DOMAIN}|{issued}|{random}|{difficulty}|{ip_bucket}").as_bytes());
        hex::encode(mac.finalize().into_bytes())
    }

    /// Difficulty for a global issuance count in the current window.
    pub fn difficulty_for_load(base: u8, issued_this_minute: u32) -> u8 {
        let extra = match issued_this_minute {
            0..=120 => 0,
            121..=480 => 1,
            481..=1920 => 2,
            _ => 3,
        };
        base.saturating_add(extra).min(MAX_DIFFICULTY)
    }

    /// Issue a challenge for `ip_bucket`, or `None` when this IP exceeded its
    /// per-minute rate (or too many IPs are being tracked).
    pub fn issue(&self, ip_bucket: &str) -> Option<(String, u8)> {
        self.issue_at(ip_bucket, now_unix())
    }

    fn issue_at(&self, ip_bucket: &str, now: u64) -> Option<(String, u8)> {
        let difficulty = {
            let mut window = self.window.lock().ok()?;
            if now.saturating_sub(window.started_unix) >= WINDOW.as_secs() {
                *window = IssueWindow {
                    started_unix: now,
                    ..IssueWindow::default()
                };
            }
            let tracked = window.per_ip.len();
            let count = match window.per_ip.get_mut(ip_bucket) {
                Some(count) => count,
                None if tracked >= MAX_TRACKED_IPS => return None,
                None => window.per_ip.entry(ip_bucket.to_string()).or_insert(0),
            };
            if *count >= self.per_ip_per_minute {
                return None;
            }
            *count += 1;
            window.total = window.total.saturating_add(1);
            Self::difficulty_for_load(self.base_difficulty, window.total)
        };

        let mut random = [0u8; 16];
        rand::rngs::OsRng.fill_bytes(&mut random);
        let random = hex::encode(random);
        let tag = self.tag(now, &random, difficulty, ip_bucket);
        Some((
            format!("{VERSION}.{now}.{random}.{difficulty}.{tag}"),
            difficulty,
        ))
    }

    /// Verify the tag, freshness, IP binding and work of a solved challenge.
    pub fn verify(
        &self,
        challenge: &str,
        nonce: &str,
        ip_bucket: &str,
    ) -> Result<VerifiedChallenge, PowError> {
        self.verify_at(challenge, nonce, ip_bucket, now_unix())
    }

    fn verify_at(
        &self,
        challenge: &str,
        nonce: &str,
        ip_bucket: &str,
        now: u64,
    ) -> Result<VerifiedChallenge, PowError> {
        if challenge.len() > 256 || nonce.is_empty() || nonce.len() > 128 {
            return Err(PowError::Malformed);
        }
        let parts: Vec<&str> = challenge.split('.').collect();
        let [version, issued, random, difficulty, tag] = parts.as_slice() else {
            return Err(PowError::Malformed);
        };
        if *version != VERSION
            || random.len() != 32
            || !random.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(PowError::Malformed);
        }
        let issued: u64 = issued.parse().map_err(|_| PowError::Malformed)?;
        let difficulty: u8 = difficulty.parse().map_err(|_| PowError::Malformed)?;
        if difficulty > MAX_DIFFICULTY {
            return Err(PowError::Malformed);
        }

        let expected = self.tag(issued, random, difficulty, ip_bucket);
        if !constant_time_eq(expected.as_bytes(), tag.as_bytes()) {
            return Err(PowError::BadTag);
        }
        if issued > now.saturating_add(5) {
            return Err(PowError::FromTheFuture);
        }
        if now.saturating_sub(issued) >= self.ttl.as_secs() {
            return Err(PowError::Expired);
        }
        if !work_satisfies(challenge, nonce, difficulty) {
            return Err(PowError::InsufficientWork);
        }
        Ok(VerifiedChallenge {
            challenge_id: hex::encode(Sha256::digest(challenge.as_bytes())),
        })
    }
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0u8, |difference, (a, b)| difference | (a ^ b))
        == 0
}

/// `SHA-256(challenge || nonce)` has at least `difficulty` leading zero bits.
pub fn work_satisfies(challenge: &str, nonce: &str, difficulty: u8) -> bool {
    let mut hasher = Sha256::new();
    hasher.update(challenge.as_bytes());
    hasher.update(nonce.as_bytes());
    let hash = hasher.finalize();
    let mut leading_zeros = 0u32;
    for byte in hash.iter() {
        if *byte == 0 {
            leading_zeros += 8;
        } else {
            leading_zeros += byte.leading_zeros();
            break;
        }
        if leading_zeros >= u32::from(difficulty) {
            break;
        }
    }
    leading_zeros >= u32::from(difficulty)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_issuer(per_ip: u32) -> PowIssuer {
        PowIssuer::new([7u8; 32], Duration::from_secs(120), 8, per_ip)
    }

    fn solve(challenge: &str, difficulty: u8) -> String {
        (0u64..)
            .map(|n| n.to_string())
            .find(|nonce| work_satisfies(challenge, nonce, difficulty))
            .unwrap()
    }

    #[test]
    fn a_solved_challenge_verifies_once_per_id() {
        let issuer = make_issuer(10);
        let (challenge, difficulty) = issuer.issue_at("198.51.100.7", 1_000).unwrap();
        let nonce = solve(&challenge, difficulty);
        let verified = issuer
            .verify_at(&challenge, &nonce, "198.51.100.7", 1_010)
            .unwrap();
        assert_eq!(verified.challenge_id.len(), 64);
        // The id is stable, so the durable single-use record catches replays.
        let again = issuer
            .verify_at(&challenge, &nonce, "198.51.100.7", 1_011)
            .unwrap();
        assert_eq!(again, verified);
    }

    #[test]
    fn tampering_ip_binding_and_expiry_are_rejected() {
        let issuer = make_issuer(10);
        let (challenge, difficulty) = issuer.issue_at("198.51.100.7", 1_000).unwrap();
        let nonce = solve(&challenge, difficulty);

        assert_eq!(
            issuer.verify_at(&challenge, &nonce, "203.0.113.9", 1_010),
            Err(PowError::BadTag),
            "a challenge is bound to the requesting IP bucket"
        );
        assert_eq!(
            issuer.verify_at(&challenge, &nonce, "198.51.100.7", 1_120),
            Err(PowError::Expired)
        );

        // Lowering the difficulty breaks the tag.
        let mut parts: Vec<String> = challenge.split('.').map(str::to_string).collect();
        parts[3] = "1".to_string();
        let downgraded = parts.join(".");
        assert_eq!(
            issuer.verify_at(&downgraded, &solve(&downgraded, 1), "198.51.100.7", 1_010),
            Err(PowError::BadTag)
        );

        // A challenge from another key is rejected.
        let other = PowIssuer::new([9u8; 32], Duration::from_secs(120), 8, 10);
        assert_eq!(
            other.verify_at(&challenge, &nonce, "198.51.100.7", 1_010),
            Err(PowError::BadTag)
        );

        for garbage in ["", "v1", "v2.1.2.3.4", "v1.x.y.z.w", &"a".repeat(300)] {
            assert_eq!(
                issuer.verify_at(garbage, "1", "198.51.100.7", 1_010),
                Err(PowError::Malformed)
            );
        }
    }

    #[test]
    fn insufficient_work_is_rejected() {
        let issuer = PowIssuer::new([7u8; 32], Duration::from_secs(120), 20, 10);
        let (challenge, difficulty) = issuer.issue_at("198.51.100.7", 1_000).unwrap();
        let wrong = (0u64..)
            .map(|n| n.to_string())
            .find(|nonce| !work_satisfies(&challenge, nonce, difficulty))
            .unwrap();
        assert_eq!(
            issuer.verify_at(&challenge, &wrong, "198.51.100.7", 1_001),
            Err(PowError::InsufficientWork)
        );
    }

    #[test]
    fn issuance_is_rate_limited_per_ip_and_bounded() {
        let issuer = make_issuer(3);
        for _ in 0..3 {
            assert!(issuer.issue_at("198.51.100.7", 1_000).is_some());
        }
        assert!(issuer.issue_at("198.51.100.7", 1_030).is_none());
        assert!(issuer.issue_at("203.0.113.9", 1_030).is_some());
        // A new window resets the per-IP count.
        assert!(issuer.issue_at("198.51.100.7", 1_061).is_some());

        let bounded = make_issuer(1);
        for index in 0..MAX_TRACKED_IPS {
            assert!(bounded.issue_at(&format!("ip-{index}"), 5_000).is_some());
        }
        assert!(
            bounded.issue_at("one-more", 5_001).is_none(),
            "the tracked-IP map never grows past its cap"
        );
    }

    #[test]
    fn difficulty_rises_with_load() {
        assert_eq!(PowIssuer::difficulty_for_load(20, 1), 20);
        assert_eq!(PowIssuer::difficulty_for_load(20, 200), 21);
        assert_eq!(PowIssuer::difficulty_for_load(20, 1_000), 22);
        assert_eq!(PowIssuer::difficulty_for_load(20, 100_000), 23);
        assert_eq!(PowIssuer::difficulty_for_load(25, 100_000), MAX_DIFFICULTY);
    }

    #[test]
    fn work_check_counts_leading_zero_bits() {
        let challenge = "fixed-challenge";
        let nonce = solve(challenge, 12);
        assert!(work_satisfies(challenge, &nonce, 12));
        assert!(!work_satisfies("another-challenge", &nonce, 255));
    }
}
