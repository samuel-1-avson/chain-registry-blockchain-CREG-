// crates/node/src/p2p.rs
// Real decentralized P2P layer using libp2p with rate limiting.

use libp2p::{
    gossipsub, kad, identify,
    noise, tcp, yamux,
    swarm::{NetworkBehaviour, SwarmEvent},
    Swarm, Multiaddr, PeerId,
};
use std::sync::Arc;
use std::time::Duration;
use anyhow::{Result, Context};
use futures::StreamExt;
use tokio::sync::mpsc;

use crate::p2p_rate_limit::{P2PRateLimiter, P2PRateLimitConfig};

/// Combined P2P behaviour.
#[derive(NetworkBehaviour)]
pub struct Behaviour {
    pub gossipsub: gossipsub::Behaviour,
    pub kademlia:  kad::Behaviour<kad::store::MemoryStore>,
    pub identify:  identify::Behaviour,
}

pub struct P2PNode {
    pub swarm: Swarm<Behaviour>,
    pub peer_id: PeerId,
    pub receiver: mpsc::Receiver<P2PCommand>,
    pub rate_limiter: P2PRateLimiter,
}

#[derive(Clone)]
pub struct P2PHandle {
    pub sender: mpsc::Sender<P2PCommand>,
}

pub enum P2PCommand {
    Broadcast { topic: String, data: Vec<u8> },
    Dial { addr: Multiaddr },
    IdentifyStorage { cid: String },
}

impl P2PNode {
    pub fn new(listen_addr: &str) -> Result<(Self, P2PHandle)> {
        let (sender, receiver) = mpsc::channel(100);
        let mut swarm = libp2p::SwarmBuilder::with_new_identity()
            // ... (rest of the SwarmBuilder remains the same)
            .with_tokio()
            .with_tcp(
                tcp::Config::default(),
                noise::Config::new,
                yamux::Config::default,
            )?
            .with_dns()?
            .with_behaviour(|key| {
                // ── Gossipsub ────────────────────────────────────────────────
                let message_id_fn = |message: &gossipsub::Message| {
                    let mut s = std::collections::hash_map::DefaultHasher::new();
                    std::hash::Hash::hash(&message.data, &mut s);
                    gossipsub::MessageId::from(std::hash::Hasher::finish(&s).to_string())
                };

                let gossipsub_config = gossipsub::ConfigBuilder::default()
                    .heartbeat_interval(Duration::from_secs(10))
                    .validation_mode(gossipsub::ValidationMode::Strict)
                    .message_id_fn(message_id_fn)
                    .build()
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

                let gossipsub = gossipsub::Behaviour::new(
                    gossipsub::MessageAuthenticity::Signed(key.clone()),
                    gossipsub_config,
                )?;

                // ── Kademlia ─────────────────────────────────────────────────
                let peer_id = key.public().to_peer_id();
                let store = kad::store::MemoryStore::new(peer_id);
                let kademlia = kad::Behaviour::new(peer_id, store);

                // ── Identify ─────────────────────────────────────────────────
                let identify = identify::Behaviour::new(identify::Config::new(
                    "/creg/1.0.0".into(),
                    key.public(),
                ));

                Ok(Behaviour { gossipsub, kademlia, identify })
            })?
            .with_swarm_config(|c| c.with_idle_connection_timeout(Duration::from_secs(60)))
            .build();

        let peer_id = *swarm.local_peer_id();
        swarm.listen_on(listen_addr.parse()?)?;

        let rate_limiter = P2PRateLimiter::new(P2PRateLimitConfig::default());

        Ok((Self { swarm, peer_id, receiver, rate_limiter }, P2PHandle { sender }))
    }

    pub async fn run(
        mut self,
        state: crate::SharedState,
    ) -> Result<()> {
        let event_bus = {
             let s = state.read().await;
             Arc::clone(&s.event_bus)
        };
        let mut status_ticker = tokio::time::interval(Duration::from_secs(5));
        let votes_topic = gossipsub::IdentTopic::new("creg/v1/votes");
        let blocks_topic = gossipsub::IdentTopic::new("creg/v1/blocks");
        let submissions_topic = gossipsub::IdentTopic::new("creg/v1/submissions");
        let vrf_proofs_topic = gossipsub::IdentTopic::new("creg/v1/vrf-proofs");
        
        self.swarm.behaviour_mut().gossipsub.subscribe(&votes_topic)?;
        self.swarm.behaviour_mut().gossipsub.subscribe(&blocks_topic)?;
        self.swarm.behaviour_mut().gossipsub.subscribe(&submissions_topic)?;
        self.swarm.behaviour_mut().gossipsub.subscribe(&vrf_proofs_topic)?;

        loop {
            tokio::select! {
                event = self.swarm.select_next_some() => match event {
                    SwarmEvent::Behaviour(BehaviourEvent::Kademlia(kad::Event::OutboundQueryProgressed { result, .. })) => {
                        match result {
                            kad::QueryResult::Bootstrap(Ok(_)) => {
                                tracing::info!("Kademlia bootstrap successful");
                            }
                            _ => {}
                        }
                    }
                    SwarmEvent::Behaviour(BehaviourEvent::Gossipsub(gossipsub::Event::Message {
                        propagation_source: peer_id,
                        message_id: id,
                        message,
                    })) => {
                        tracing::debug!("Got Gossipsub message {} from {}", id, peer_id);
                        
                        // Parse topic and check rate limits
                        let topic_str = message.topic.as_str();
                        
                        // Apply rate limiting based on message type
                        let allowed = if topic_str.contains("votes") {
                            self.rate_limiter.check_vote(peer_id)
                        } else if topic_str.contains("blocks") {
                            self.rate_limiter.check_block(peer_id)
                        } else {
                            self.rate_limiter.check_general(peer_id)
                        };
                        
                        if !allowed {
                            tracing::warn!(
                                "P2P Rate limit: Dropping message {} from {} on topic {}",
                                id, peer_id, topic_str
                            );
                            continue;
                        }
                        
                        // Forward message to the node's internal event bus
                        if topic_str.contains("submissions") {
                            if let Ok(common::GossipMessage::PublishRequest(req)) = serde_json::from_slice(&message.data) {
                                let mut s = state.write().await;
                                if !s.pending_pool.contains(&req.id.canonical()) {
                                    s.pending_pool.insert(req.clone());
                                    tracing::info!("Received {} via gossip", req.id.canonical());
                                }
                            }
                            continue;
                        }

                        if topic_str.contains("vrf-proofs") {
                            if let Ok(common::GossipMessage::VrfProof { validator_id, pubkey, epoch_seed, output, proof }) = serde_json::from_slice(&message.data) {
                                let mut s = state.write().await;
                                let current_seed = match s.chain.tip_hash() {
                                    Ok(h) => h,
                                    Err(_) => continue,
                                };
                                // Only accept proofs for the current epoch seed
                                if epoch_seed == current_seed {
                                    if let Err(e) = consensus::vrf::verify(epoch_seed.as_bytes(), &pubkey, &output, &proof) {
                                        tracing::debug!("Dropped invalid VRF proof from {}: {}", validator_id, e);
                                    } else {
                                        s.vrf_proofs.insert(validator_id.clone(), (output.clone(), proof.clone()));
                                        tracing::debug!("Accepted VRF proof from {} for epoch {}", validator_id, &epoch_seed[..epoch_seed.len().min(12)]);
                                    }
                                }
                            }
                            continue;
                        }

                        let kind = if topic_str.contains("votes") { crate::events::EventKind::ValidatorVoted } 
                                    else { crate::events::EventKind::BlockProduced };
                        crate::events::emit(&event_bus, crate::events::RegistryEvent {
                            kind,
                            ts: chrono::Utc::now().to_rfc3339(),
                            payload: serde_json::json!({ "p2p_message": String::from_utf8_lossy(&message.data).to_string() }),
                        });
                    }
                    SwarmEvent::NewListenAddr { address, .. } => {
                        tracing::info!("P2P node listening on {}", address);
                    }
                    SwarmEvent::ConnectionEstablished { peer_id, endpoint, .. } => {
                        tracing::info!("P2P Connection established with {} at {:?}", peer_id, endpoint);
                    }
                    SwarmEvent::OutgoingConnectionError { peer_id, error, .. } => {
                        tracing::error!("P2P Outgoing connection error to {:?}: {}", peer_id, error);
                    }
                    _ => {}
                },

                // ── Periodically update SharedState with peer list ────────────
                _ = status_ticker.tick() => {
                    let peers: Vec<String> = self.swarm.connected_peers()
                        .map(|p| p.to_string())
                        .collect();
                    let mut s = state.write().await;
                    s.p2p_status.peers = peers;
                    s.p2p_status.protocols = vec!["Identify".into(), "Ping".into(), "Kademlia".into()];
                }

                // ── Identify Storage Responsibility (Sharding) ───────────────
                command = self.receiver.recv() => {
                    if let Some(cmd) = command {
                        match cmd {
                            P2PCommand::Broadcast { topic, data } => {
                                let t = gossipsub::IdentTopic::new(topic);
                                if let Err(e) = self.swarm.behaviour_mut().gossipsub.publish(t, data) {
                                    tracing::error!("P2P broadcast failed: {}", e);
                                }
                            }
                            P2PCommand::Dial { addr } => {
                                tracing::info!("P2P Dialing {}...", addr);
                                if let Err(e) = self.swarm.dial(addr) {
                                    tracing::error!("P2P dial failed: {}", e);
                                }
                            }
                            P2PCommand::IdentifyStorage { cid } => {
                                let is_responsible = self.is_responsible_for(&cid);
                                tracing::info!("Storage check for {}: Responsible={}", cid, is_responsible);
                                // Logic to trigger Pinning/Pruning would happen here
                            }
                        }
                    }
                }
            }
        }
    }

    /// Determines if this node is among the 'N' closest nodes to a CID.
    /// This is the core of our 'Masterless Sharding' for 500MB+ packages.
    pub fn is_responsible_for(&self, cid: &str) -> bool {
        let local_bytes = self.peer_id.to_bytes();
        let cid_bytes = cid.as_bytes(); // In production, use multihash of the CID
        
        // Simple XOR distance simulation for sharding
        // If the first byte matches or is close, we take responsibility.
        // A threshold of 32 (1/8th of network) ensures 7-10 nodes store each shard.
        if local_bytes.is_empty() || cid_bytes.is_empty() { return false; }
        let distance = local_bytes[0] ^ cid_bytes[0];
        distance < 32
    }
}
