# Implementation Plan: Option B - P2P Gossip Network & Global Topology Hardening

## Problem & Background

In a decentralized validator network operating at scale (10,000+ nodes), the P2P layer is vulnerable to three major network attack vectors:
1. **Sybil & Eclipse Attacks**: Malicious actors spinning up hundreds of nodes on the same cloud IP subnet (`/24`) to monopolize a validator's GossipSub mesh connections and delay block or vote propagation.
2. **Spam & Toxic Message Delivery**: Malicious or buggy peers spamming invalid signatures or corrupted packages, consuming CPU during verification.
3. **Bootnode Partitioning & Churn**: Network partitions if initial bootnodes go offline or experience flapping connectivity.

---

## Proposed Hardening Architecture

### 1. Dynamic Peer Scoring & Penalty Matrix ([`crates/node/src/p2p_peer_score.rs`](../crates/node/src/p2p_peer_score.rs))
- **`PeerScoreTracker`**: Tracks multi-dimensional metrics per peer:
  - Valid message delivery (+ score reward).
  - Invalid signature / corrupted payload deliveries (- severe score penalty).
  - Rate limit violations (- moderate penalty).
  - Message delivery latency / responsiveness.
- **Dynamic Action Gates**:
  - `Score >= 0.0`: Healthy peer (full mesh participation).
  - `-50.0 <= Score < 0.0`: Graylisted (gossip relayed with rate limits).
  - `Score < -50.0`: Disconnect & ban peer for escalating cooldown periods.

### 2. Anti-Sybil Subnet Ingress Bounding ([`crates/node/src/p2p_subnet_filter.rs`](../crates/node/src/p2p_subnet_filter.rs))
- **`SubnetConnectionFilter`**:
  - Groups inbound connections by `/24` IPv4 and `/48` IPv6 subnets.
  - Limits maximum concurrent active connections per subnet (default: 3 per `/24` subnet).
  - Guarantees diversity in GossipSub mesh topology to neutralize eclipse attacks.
  - Exempts authenticated bonded validators from subnet limits to maintain validator mesh connectivity.

### 3. Resilient Bootnode Manager ([`crates/node/src/p2p_bootnodes.rs`](../crates/node/src/p2p_bootnodes.rs))
- **`BootnodeManager`**:
  - Manages primary and fallback bootstrap multiaddrs.
  - Implements exponential backoff with jitter on failed dials.
  - Automatically triggers Kademlia DHT routing table refreshes when active peer count drops below minimum threshold.

---

## Proposed Changes

### `crates/node`

#### [NEW] [`crates/node/src/p2p_peer_score.rs`](../crates/node/src/p2p_peer_score.rs)
- Implements `PeerScoreTracker`, `PeerMetrics`, and penalty calculations.

#### [NEW] [`crates/node/src/p2p_subnet_filter.rs`](../crates/node/src/p2p_subnet_filter.rs)
- Implements `SubnetConnectionFilter` with `/24` and `/48` IP parsing and validator exemption.

#### [NEW] [`crates/node/src/p2p_bootnodes.rs`](../crates/node/src/p2p_bootnodes.rs)
- Implements `BootnodeManager` with health tracking and auto-redial.

#### [MODIFY] [`crates/node/src/lib.rs`](../crates/node/src/lib.rs)
- Expose new modules and integrate with node P2P state.

---

## Verification Plan

### Automated Tests
1. **P2P modules unit tests**:
   ```bash
   cargo test -p chain-registry-node --lib p2p
   ```
2. **Full node test suite**:
   ```bash
   cargo test -p chain-registry-node --lib
   ```
3. **Consensus & Validator integration suites**:
   ```bash
   cargo test -p consensus
   cargo test -p validator
   ```
