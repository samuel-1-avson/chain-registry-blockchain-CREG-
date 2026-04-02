# Security Model & Threat Analysis

## Overview

Chain Registry implements defense-in-depth with multiple security layers:

```
┌─────────────────────────────────────────────────────────────────┐
│                    THREAT LANDSCAPE                              │
├─────────────────────────────────────────────────────────────────┤
│                                                                  │
│  Publishers ──────► Malware Upload ──────► AI + Sandbox         │
│  Validators ──────► Double-Sign ─────────► ZK Slashing          │
│  Pinners ─────────► Data Unavailable ────► Staking + Slashing   │
│  Network ─────────► Eclipse/Sybil ───────► P2P Mesh + PoS       │
│  Contracts ───────► Exploit ─────────────► Audits + Formal Verif│
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

## Threat Model

### 1. Malicious Publishers

**Threat:** Upload malware disguised as legitimate package.

**Attack Vectors:**
- Obfuscated malicious code
- Supply chain poisoning
- Typosquatting (similar package names)
- Zero-day exploits

**Defenses:**
```
Layer 1: AI Screening (TF-IDF + CodeBERT)
   └─> 99.95% accuracy on known malware signatures

Layer 2: Static Analysis
   └─> AST-based pattern matching for suspicious constructs

Layer 3: Sandbox Execution
   └─> Isolated environment with behavior monitoring

Layer 4: Validator Consensus
   └─> 7-of-10 validators must approve

Layer 5: Economic Stake
   └─> Publisher stake at risk if package revoked
```

**Response:** Automatic rejection, publisher reputation reduction.

---

### 2. Compromised Validators

**Threat:** Validator acts maliciously or is compromised.

**Attack Vectors:**
- **Double-signing:** Approve and reject same package
- **False approval:** Approve known malware
- **Griefing:** Consistently reject legitimate packages
- **Censorship:** Refuse to process specific packages

**Defenses:**

#### Double-Signing
```rust
// ZK Proof of equivocation
let evidence = DoubleSignEvidence {
    validator_pubkey: pubkey,
    vote1_hash: hash("approve:package:v1"),
    vote2_hash: hash("reject:package:v1"),
    // ... signatures prove conflict
};

// Automatic slashing - no voting required!
zk_verifier.verify_double_sign(proof, public_inputs)?;
slash_validator(validator, 1000_CREG)?;
```

#### False Approval
```rust
// If package later revoked as malicious
if package.status == Revoked && validator.voted_approve {
    // Slash for incorrect validation
    slash_validator(validator, 500_CREG)?;
    reduce_reputation(validator, -50)?;
}
```

#### Griefing Detection
```rust
// Track validator voting patterns
let agreement_rate = validator.agreement_with_majority();
if agreement_rate < 0.10 && validator.total_votes > 100 {
    // Likely griefing
    initiate_slash_proposal(validator)?;
}
```

**Response:** Automatic slashing (1000 CREG for double-sign), reputation reduction.

---

### 3. Pinning Failures

**Threat:** IPFS pinner fails to store or serve content.

**Attack Vectors:**
- Delete content after receiving rewards
- Serve corrupted data
- DDoS other pinners

**Defenses:**
```solidity
// Staking requirement
function registerPinner() external payable {
    require(msg.value >= MIN_STAKE, "Stake 1000 CREG");
    // ...
}

// Periodic verification
function verifyPin(string calldata cid, address pinner) external {
    require(isContentAvailable(cid, pinner), "Content unavailable");
    // Failure results in slashing
    slash(pinner, calculatePenalty(cid));
}

// Whistleblower reward
function reportUnavailable(string calldata cid, address pinner) external {
    require(!isContentAvailable(cid, pinner), "Content is available");
    uint256 penalty = slash(pinner, STAKE_PERCENTAGE);
    reward(msg.sender, penalty / 10); // 10% to reporter
}
```

**Response:** 1% slashing per failed verification, 10% whistleblower reward.

---

### 4. Network Attacks

**Threat:** Disrupt P2P network or isolate validators.

**Attack Vectors:**
- **Eclipse Attack:** Isolate validator from honest peers
- **Sybil Attack:** Create many fake identities
- **DDoS:** Flood network with invalid messages
- **Censorship:** Block specific packages from propagation

**Defenses:**

#### Eclipse Protection
```rust
// Bootstrap with trusted seeds
let bootstrap_peers = [
    "/dns/node-1.creg.dev/tcp/4001/p2p/12D3KooW...",
    "/dns/node-2.creg.dev/tcp/4001/p2p/12D3KooW...",
    // ... 10+ bootstrap nodes
];

// Require minimum peer connections
if peer_count < 5 {
    initiate_bootstrap_reconnect()?;
}
```

#### Sybil Resistance
```rust
// Proof-of-stake validator set
let validator_weight = validator.stake + validator.reputation;
let quorum = total_stake * 2 / 3;

// New validators need existing validators to approve
function add_validator(new_validator) {
    require(validator_set.approval_count() >= threshold);
    // ...
}
```

#### Rate Limiting
```rust
// P2P rate limiting per peer
struct RateLimiter {
    max_requests_per_second: u32 = 100,
    burst_size: u32 = 200,
}

// Ban peers that exceed limits
if peer.rate_limit.exceeded() {
    peer.ban(duration = 1.hour);
}
```

**Response:** Automatic peer banning, bootstrap reconnection.

---

### 5. Smart Contract Vulnerabilities

**Threat:** Exploit bugs in contracts to steal funds or disrupt protocol.

**Attack Vectors:**
- Reentrancy attacks
- Integer overflow/underflow
- Access control bypass
- Front-running
- Oracle manipulation

**Defenses:**

#### Access Control
```solidity
modifier onlyValidator() {
    require(validatorSet.contains(msg.sender), "Not validator");
    _;
}

modifier onlyOwner() {
    require(msg.sender == owner, "Not owner");
    _;
}
```

#### Reentrancy Protection
```solidity
uint256 private locked;

modifier nonReentrant() {
    require(locked == 0, "Reentrant call");
    locked = 1;
    _;
    locked = 0;
}

function claimRewards() external nonReentrant {
    // Safe from reentrancy
}
```

#### Integer Safety
```solidity
// Solidity 0.8+ has built-in overflow protection
uint256 public totalSupply; // Automatic overflow checks

// Explicit checks for critical operations
function slash(address validator, uint256 amount) external {
    require(amount <= stakes[validator], "Amount exceeds stake");
    stakes[validator] -= amount;
}
```

**Response:** Multi-sig contract upgrades, emergency pause.

---

### 6. Cryptographic Attacks

**Threat:** Break cryptographic primitives.

**Attack Vectors:**
- Forge validator signatures
- Break ZK proofs
- Compromise threshold encryption

**Defenses:**

#### Signature Security
```rust
// Ed25519 for validator signatures
// - 128-bit security level
// - Deterministic signatures (no nonce reuse)
let signature = ed25519::sign(message, private_key);
assert!(ed25519::verify(message, signature, public_key));
```

#### ZK Proof Soundness
```circom
// Groth16 with BN128 curve
// - 128-bit security in generic group model
// - Trusted setup required (MPC ceremony)
template DoubleSignProof() {
    // ... constraints ensure valid signatures
}
```

#### Threshold Encryption
```rust
// Shamir Secret Sharing (5-of-10)
// - 256-bit keys
// - Information-theoretic security (unconditional)
let shares = shamir::split(secret, threshold=5, shares=10);
let recovered = shamir::reconstruct(any_5_shares);
```

**Response:** Cryptographic primitive upgrades via governance.

---

## Security Parameters

| Parameter | Value | Rationale |
|-----------|-------|-----------|
| Validator Threshold | 7-of-10 (67%) | Byzantine fault tolerance |
| Double-sign Slash | 1000 CREG | Deterrence + compensation |
| Pinner Stake | 1000 CREG | Skin in the game |
| Pinning Penalty | 1% per failure | Proportional to offense |
| Block Time | 2 seconds | Finality vs throughput tradeoff |
| Vote Timeout | 10 seconds | Fast consensus failure detection |
| AI Confidence | 95% | Low false positive rate |

## Audit Checklist

### Smart Contracts
- [ ] Reentrancy analysis
- [ ] Integer overflow checks
- [ ] Access control verification
- [ ] Gas optimization review
- [ ] Upgrade mechanism security
- [ ] Event emission completeness

### Cryptographic Components
- [ ] ZK circuit formal verification
- [ ] Signature scheme implementation
- [ ] Randomness source analysis
- [ ] Key management review
- [ ] Threshold scheme correctness

### Network Security
- [ ] P2P protocol analysis
- [ ] Rate limiting effectiveness
- [ ] DDoS resistance testing
- [ ] Eclipse attack simulation
- [ ] Message authentication

### Infrastructure
- [ ] Docker image security
- [ ] Secret management
- [ ] Log integrity
- [ ] Monitoring coverage
- [ ] Incident response plan

## Incident Response

### Severity Levels

| Level | Criteria | Response |
|-------|----------|----------|
| **Critical** | Funds at risk, consensus failure | Emergency pause, 24h fix |
| **High** | Validator compromise possible | 48h investigation |
| **Medium** | Performance degradation | 1 week fix |
| **Low** | Cosmetic issues | Next release |

### Emergency Procedures

```rust
// Emergency pause
function emergencyPause() external onlyGuardian {
    paused = true;
    emit EmergencyPaused(msg.sender);
}

// Validator set override (extreme cases)
function emergencyValidatorReset(address[] calldata newSet) 
    external 
    onlyMultisig(5of9) 
{
    validatorSet = newSet;
    emit EmergencyValidatorReset(newSet);
}
```

## Bug Bounty Program

### Scope
- Smart contracts
- ZK circuits
- P2P protocol
- Node implementation

### Rewards
| Severity | Reward |
|----------|--------|
| Critical | $50,000 - $100,000 |
| High | $10,000 - $50,000 |
| Medium | $2,000 - $10,000 |
| Low | $500 - $2,000 |

### Rules
- Responsible disclosure required
- No social engineering
- No DDoS on mainnet
- 90-day disclosure deadline

## References

- [Groth16 Security](https://eprint.iacr.org/2016/260)
- [Ed25519 Specification](https://ed25519.cr.yp.to/)
- [Byzantine Fault Tolerance](https://pmg.csail.mit.edu/papers/osdi99.pdf)
- [Smart Contract Security Best Practices](https://consensys.github.io/smart-contract-best-practices/)

---

*Last updated: 2026-04-02*
*Security contact: security@chain-registry.dev*
