// crates/node/src/events.rs
// WebSocket event stream — clients subscribe to GET /v1/events and receive
// a real-time feed of registry events as newline-delimited JSON.
//
// Event types:
//   package.submitted   — new package entered the pending pool
//   package.verified    — package cleared consensus and is on chain
//   package.rejected    — package failed validation
//   package.revoked     — previously verified package was revoked
//   block.produced      — new block appended to the chain
//   validator.voted     — a validator cast a PBFT vote
//
// Uses Axum's SSE (Server-Sent Events) transport — compatible with
// EventSource in browsers and `curl --no-buffer` in terminals.
// SSE is simpler than WebSocket for a one-way event feed.

use axum::{
    extract::State,
    response::sse::{Event, Sse},
};
use chrono::Utc;
use futures::stream::{self, Stream};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashSet, VecDeque},
    convert::Infallible,
    fmt,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::broadcast;
use tokio_stream::StreamExt;

/// Capacity of the broadcast channel.
/// Old events are dropped when the channel is full (slow consumers).
pub const EVENT_CHANNEL_CAPACITY: usize = 256;

/// A single registry event emitted on the SSE stream.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryEvent {
    /// Event type identifier.
    pub kind: EventKind,
    /// ISO-8601 timestamp.
    pub ts: String,
    /// Event-specific payload.
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    PackageSubmitted,
    PackageVerified,
    PackageRejected,
    PackageRevoked,
    BlockProduced,
    ValidatorVoted,
}

impl fmt::Display for EventKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Produce the same snake_case string that serde outputs.
        let s = match self {
            EventKind::PackageSubmitted => "package_submitted",
            EventKind::PackageVerified => "package_verified",
            EventKind::PackageRejected => "package_rejected",
            EventKind::PackageRevoked => "package_revoked",
            EventKind::BlockProduced => "block_produced",
            EventKind::ValidatorVoted => "validator_voted",
        };
        f.write_str(s)
    }
}

impl RegistryEvent {
    pub fn package_submitted(canonical: &str, publisher_pubkey: &str) -> Self {
        Self {
            kind: EventKind::PackageSubmitted,
            ts: Utc::now().to_rfc3339(),
            payload: serde_json::json!({
                "canonical":        canonical,
                "publisher_pubkey": &publisher_pubkey[..publisher_pubkey.len().min(16)],
            }),
        }
    }

    pub fn package_verified(canonical: &str, block_hash: &str, validator_count: usize) -> Self {
        Self {
            kind: EventKind::PackageVerified,
            ts: Utc::now().to_rfc3339(),
            payload: serde_json::json!({
                "canonical":       canonical,
                "block_hash":      block_hash,
                "validator_count": validator_count,
            }),
        }
    }

    pub fn package_rejected(canonical: &str, reason: &str) -> Self {
        Self {
            kind: EventKind::PackageRejected,
            ts: Utc::now().to_rfc3339(),
            payload: serde_json::json!({
                "canonical": canonical,
                "reason":    reason,
            }),
        }
    }

    pub fn package_revoked(canonical: &str, reason: &str, revoked_by: &str) -> Self {
        Self {
            kind: EventKind::PackageRevoked,
            ts: Utc::now().to_rfc3339(),
            payload: serde_json::json!({
                "canonical":  canonical,
                "reason":     reason,
                "revoked_by": revoked_by,
            }),
        }
    }

    pub fn block_produced(height: u64, hash: &str, tx_count: usize) -> Self {
        Self {
            kind: EventKind::BlockProduced,
            ts: Utc::now().to_rfc3339(),
            payload: serde_json::json!({
                "height":   height,
                "hash":     hash,
                "tx_count": tx_count,
            }),
        }
    }

    pub fn validator_voted(validator_id: &str, canonical: &str, approved: bool) -> Self {
        Self {
            kind: EventKind::ValidatorVoted,
            ts: Utc::now().to_rfc3339(),
            payload: serde_json::json!({
                "validator_id": validator_id,
                "canonical":    canonical,
                "approved":     approved,
            }),
        }
    }

    pub fn with_committed_block(mut self, block: &common::Block) -> Self {
        if let Some(payload) = self.payload.as_object_mut() {
            payload.insert("height".into(), block.header.height.into());
            payload.insert("block_hash".into(), block.hash().into());
            payload.insert("block_phase".into(), "committed".into());
        }
        self
    }
}

/// Shared broadcast sender — clone this to emit events from anywhere in the node.
pub type EventBus = Arc<broadcast::Sender<RegistryEvent>>;

pub fn new_event_bus() -> EventBus {
    let (tx, _) = broadcast::channel(EVENT_CHANNEL_CAPACITY);
    Arc::new(tx)
}

/// Emit an event, silently ignoring errors if no subscribers are connected.
pub fn emit(bus: &EventBus, event: RegistryEvent) {
    let _ = bus.send(event);
}

/// Per-process notifications shared by every clone of the canonical store.
/// Installed after startup replay, so historical blocks are never announced
/// as new commits. This is a live feed; it is not a durable event archive.
pub(crate) struct RuntimeEventEmitter {
    bus: EventBus,
    recent_commits: Mutex<(HashSet<String>, VecDeque<String>)>,
}

impl RuntimeEventEmitter {
    pub(crate) fn new(bus: EventBus) -> Self {
        Self {
            bus,
            recent_commits: Mutex::new((HashSet::new(), VecDeque::new())),
        }
    }

    pub(crate) fn emit(&self, event: RegistryEvent) {
        emit(&self.bus, event);
    }

    pub(crate) fn first_notification_for_commit(&self, hash: &str) -> bool {
        let mut recent = self
            .recent_commits
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if !recent.0.insert(hash.to_owned()) {
            return false;
        }
        recent.1.push_back(hash.to_owned());
        while recent.1.len() > EVENT_CHANNEL_CAPACITY {
            if let Some(expired) = recent.1.pop_front() {
                recent.0.remove(&expired);
            }
        }
        true
    }
}

// ── WebSocket handler ─────────────────────────────────────────────────────────

/// GET /v1/ws
/// WebSocket endpoint for ultra-low latency event streaming.
pub async fn ws_handler(
    ws: axum::extract::ws::WebSocketUpgrade,
    State(bus): State<EventBus>,
) -> axum::response::Response {
    ws.on_upgrade(move |socket| handle_ws_client(socket, bus))
}

async fn handle_ws_client(mut socket: axum::extract::ws::WebSocket, bus: EventBus) {
    let mut rx = bus.subscribe();

    // Loop to read events from the broadcast channel and send them over the websocket
    while let Ok(event) = rx.recv().await {
        let msg = match serde_json::to_string(&event) {
            Ok(s) => s,
            Err(_) => continue,
        };

        if socket
            .send(axum::extract::ws::Message::Text(msg))
            .await
            .is_err()
        {
            // Client abruptly disconnected
            break;
        }
    }
}

// ── SSE handler ───────────────────────────────────────────────────────────────

/// GET /v1/events
/// Returns a Server-Sent Events stream. Connect with:
///   curl -N http://localhost:8080/v1/events
///   new EventSource('http://localhost:8080/v1/events')
pub async fn sse_handler(
    State(bus): State<EventBus>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let rx = bus.subscribe();

    // Convert the broadcast receiver into an SSE-compatible stream.
    let stream = tokio_stream::wrappers::BroadcastStream::new(rx).filter_map(|result| {
        match result {
            Ok(event) => {
                let data = serde_json::to_string(&event).unwrap_or_default();
                let sse_event = Event::default()
                    .event(event.kind.to_string()) // snake_case via Display
                    .data(data);
                Some(Ok(sse_event))
            }
            // Lagged — subscriber fell too far behind, drop and continue.
            Err(tokio_stream::wrappers::errors::BroadcastStreamRecvError::Lagged(n)) => {
                tracing::warn!("SSE client lagged — dropped {} events", n);
                None
            }
        }
    });

    // Heartbeat: send a comment every 30s to keep the connection alive
    // through proxies that close idle SSE connections.
    let heartbeat = stream::repeat(())
        .throttle(Duration::from_secs(30))
        .map(|_| Ok(Event::default().comment("heartbeat")));

    let combined = stream::select(stream, heartbeat);

    Sse::new(combined).keep_alive(
        axum::response::sse::KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("ping"),
    )
}
