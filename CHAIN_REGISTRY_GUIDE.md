# Chain Registry — Complete System Guide

> A decentralized, Byzantine Fault Tolerant package security network for the global software supply chain.

---

## Table of Contents

1. [The Problem It Solves](#1-the-problem-it-solves)
2. [What Chain Registry Is](#2-what-chain-registry-is)
3. [How the Blockchain Works](#3-how-the-blockchain-works)
4. [System Architecture Diagram](#4-system-architecture-diagram)
5. [System Flow — Step by Step](#5-system-flow--step-by-step)
6. [Workflow Diagrams](#6-workflow-diagrams)
7. [Features](#7-features)
8. [How People Use It](#8-how-people-use-it)
9. [Why It Matters — The Importance](#9-why-it-matters--the-importance)
10. [Comparison: Before vs After](#10-comparison-before-vs-after)

---

## 1. The Problem It Solves

### The Software Supply Chain Crisis

Every developer in the world depends on open-source packages. Every time you run `npm install`, `pip install`, or `cargo add`, you are downloading and executing code written by strangers on your machine — and in production systems. This is called the **software supply chain**.

This supply chain is critically broken:

```
The Trust Chain Today (BROKEN):
─────────────────────────────────────────────────────────────

 Attacker uploads         npm/PyPI just         Your machine
 malicious package   ──►  stores it        ──►  runs it blindly
 (lodahs, not lodash)     no verification        💀 infected

─────────────────────────────────────────────────────────────
```

### Real-World Attacks That Already Happened

| Year | Attack | Impact |
|------|--------|--------|
| 2018 | `event-stream` npm package backdoored | 8 million downloads/week; targeted Bitcoin wallets |
| 2021 | SolarWinds (supply chain) | 18,000 organizations compromised, including US government |
| 2022 | `colors` and `faker` sabotage | Millions of apps broken by a single angry developer |
| 2021 | `ua-parser-js` hijacked | 7+ million downloads/week; crypto miners injected |
| 2022 | PyTorch nightly poisoning | GPU driver replaced with credential stealer |
| 2023 | XZ Utils backdoor (CVE-2024-3094) | Nearly backdoored SSH across all Linux systems |
| 2024 | Polyfill.io CDN hijack | 100,000+ websites serving malicious JavaScript |

### Root Cause: Single-Authority Trust

All existing package registries share one fatal design flaw:

```
npm says it's safe  →  it's trusted   (one authority)
npm gets hacked     →  everything is compromised
npm makes mistake   →  no second opinion
npm is slow         →  no escalation path
```

There is no independent verification. No consensus. No economic accountability. One company's database is the entire trust layer for the global software ecosystem.

### What Chain Registry Fixes

```
Chain Registry Trust Model (FIXED):
─────────────────────────────────────────────────────────────────────

 Publisher uploads     10+ Validators       2/3+ Consensus
 package          ──►  independently   ──►  required before   ──►  ✓ VERIFIED
                        analyze it           it's trusted           on chain

 One validator         Quorum rejects it    Package BLOCKED
 gets hacked     ──►   it anyway        ──►  automatically

 Bad package found     Validator who        Stake SLASHED
 after publish   ──►   approved it     ──►  (financial penalty)

─────────────────────────────────────────────────────────────────────
```

---

## 2. What Chain Registry Is

**Chain Registry** (`creg`) is a decentralized package security layer that sits transparently between developers and their existing package managers.

### Core Identity

| Property | Value |
|----------|-------|
| **Type** | Byzantine Fault Tolerant (BFT) blockchain — Layer 2 anchored to Ethereum L1 |
| **Language** | Rust (node/validator) + Solidity (contracts) |
| **Consensus** | PBFT (Practical Byzantine Fault Tolerance) |
| **Ecosystems** | npm, pip, cargo, gem, mvn |
| **Anchoring** | Ethereum L1 (permanent, immutable record) |
| **Storage** | IPFS (decentralized tarball hosting) |
| **Cryptography** | Ed25519 (signatures), AES-256-GCM (encryption), Groth16/BN254 (ZK proofs) |
| **Native Token** | CREG — Chain Registry Token (ERC-20, hard capped at 42,000,000) |
| **Staking Currency** | CREG (not ETH) — publishers stake 1 CREG, validators stake 100 CREG |
| **Validator Model** | One validator per machine — each operator runs their own independent node |

### The Simple Mental Model

```
Without Chain Registry:                  With Chain Registry:
──────────────────────                   ──────────────────────
npm install express                      npm install express
      │                                         │
      ▼                                         ▼
  npm registry                          creg shim (invisible)
  (trust blindly)                              │
      │                                         ▼ (milliseconds)
      ▼                               Check Chain Registry:
  install runs                          • Already verified? → proceed
  🎲 safe or not?                       • Revoked?          → BLOCK
                                         • Unknown?          → warn
                                              │
                                              ▼
                                         npm registry
                                         (with confidence)
```

---

## 3. How the Blockchain Works

### 3.1 The Ledger Structure

Chain Registry maintains a blockchain — a linked list of blocks, each containing a batch of verified package decisions.

```
GENESIS                BLOCK 1                BLOCK 2                BLOCK N
┌─────────────────┐    ┌─────────────────┐    ┌─────────────────┐    ┌─────────────────┐
│ height:    0    │    │ height:    1    │    │ height:    2    │    │ height:    N    │
│ prev_hash: 000  │◄───│ prev_hash: h0   │◄───│ prev_hash: h1   │◄───│ prev_hash: hN-1 │
│ merkle:    ...  │    │ merkle:    ...  │    │ merkle:    ...  │    │ merkle:    ...  │
│ proposer:  dev  │    │ proposer:  v1   │    │ proposer:  v2   │    │ proposer:  v3   │
│ timestamp: ...  │    │ timestamp: ...  │    │ timestamp: ...  │    │ timestamp: ...  │
│                 │    │                 │    │                 │    │                 │
│ transactions:   │    │ transactions:   │    │ transactions:   │    │ transactions:   │
│  (none)         │    │  Publish(...)   │    │  Publish(...)   │    │  Publish(...)   │
│                 │    │  Publish(...)   │    │  Revoke(...)    │    │  Slash(...)     │
└─────────────────┘    └─────────────────┘    └─────────────────┘    └─────────────────┘
```

Each block is **cryptographically linked** to the previous one via SHA-256 hashes. Modifying any past block invalidates all subsequent blocks — making history tamper-proof.

### 3.2 Transaction Types

The blockchain records five types of events:

```
┌──────────────────────┬─────────────────────────────────────────────────────────┐
│                          TRANSACTION TYPES                                      │
├──────────────────────┼─────────────────────────────────────────────────────────┤
│ Publish              │ A package passed validation. Now safe to install.        │
│                      │ Contains: package ID, content hash, IPFS CID,           │
│                      │ validator signatures (quorum proof)                      │
├──────────────────────┼─────────────────────────────────────────────────────────┤
│ Revoke               │ A previously verified package was found malicious        │
│                      │ Contains: canonical ID, reason, evidence hash            │
├──────────────────────┼─────────────────────────────────────────────────────────┤
│ Slash                │ A validator was penalized — CREG deducted from their     │
│                      │ stake and added to the slash pool for redistribution     │
│                      │ Contains: validator ID, CREG amount, reason              │
├──────────────────────┼─────────────────────────────────────────────────────────┤
│ ValidatorApply       │ A candidate applied to become a validator by locking     │
│                      │ 100 CREG in escrow. Status: Pending until approved.      │
│                      │ Contains: validator ID, pubkey, CREG stake amount        │
├──────────────────────┼─────────────────────────────────────────────────────────┤
│ ValidatorApproved    │ Governance approved a pending validator application.     │
│                      │ Validator is now Active and may vote in consensus.       │
├──────────────────────┼─────────────────────────────────────────────────────────┤
│ ValidatorRejected    │ Governance rejected a pending application.               │
│                      │ Applicant's full CREG stake is returned immediately.     │
├──────────────────────┼─────────────────────────────────────────────────────────┤
│ ValidatorLeave       │ An active validator initiated unbonding.                 │
│                      │ Stake locked for 7 days before withdrawal.               │
│                      │ Contains: validator ID                                   │
└──────────────────────┴─────────────────────────────────────────────────────────┘
```

### 3.3 The Consensus Mechanism — PBFT

Chain Registry uses **Practical Byzantine Fault Tolerance (PBFT)** — the same class of consensus used in enterprise blockchains (Hyperledger, Tendermint). It guarantees safety even if up to **1/3 of validators are malicious or offline**.

```
PBFT CONSENSUS — 3 PHASES
══════════════════════════════════════════════════════════════════════════

                    VRF SELECTS PRIMARY
                    (Verifiable Random Function — unpredictable,
                     prevents validators from gaming selection)
                           │
                           ▼
┌──────────────────────────────────────────────────────────────────────┐
│  PHASE 1: PRE-PREPARE                                                │
│                                                                      │
│  Primary Validator ──► broadcasts block proposal to all validators   │
│  (contains: package canonical, content hash, IPFS CID, timestamp)   │
└──────────────────────────────────────────────────────────────────────┘
                           │
                           ▼ (each validator independently)
┌──────────────────────────────────────────────────────────────────────┐
│  PHASE 2: PREPARE                                                    │
│                                                                      │
│  Each Validator runs 3-stage analysis pipeline:                      │
│    ├── Stage 1: Static code analysis (pattern matching + entropy)    │
│    ├── Stage 2: Sandbox execution (nsjail, 120s, 512MB)              │
│    └── Stage 3: Reputation assessment (publisher history)            │
│                                                                      │
│  Then broadcasts signed vote: { Approve | Reject, signature }       │
│                                                                      │
│  Waits for 2f+1 PREPARE votes  (f = ⌊n/3⌋ faulty tolerance)        │
└──────────────────────────────────────────────────────────────────────┘
                           │
                           ▼ (if quorum achieved)
┌──────────────────────────────────────────────────────────────────────┐
│  PHASE 3: COMMIT                                                     │
│                                                                      │
│  Each Validator broadcasts COMMIT message                            │
│  Waits for 2f+1 COMMIT messages                                      │
│                                                                      │
│  On success → writes Block to local chain (Sled DB)                 │
│             → broadcasts block to all peers via Gossipsub            │
│             → submits to Ethereum L1 (Registry.sol)                 │
└──────────────────────────────────────────────────────────────────────┘

Quorum math: n=10 validators → f=3 faulty → need 7 votes to finalize

Single-validator mode: When CREG_SINGLE_VALIDATOR_MODE=true (local development),
quorum = 1/1 — one vote finalizes a block. Disabled before testnet/mainnet.
```

### 3.4 The 3-Stage Validation Pipeline

Every package goes through three independent analysis stages before getting a vote:

```
                        PACKAGE TARBALL
                             │
            ┌────────────────┼────────────────┐
            │                │                │
            ▼                ▼                ▼
   ┌─────────────────┐ ┌──────────────┐ ┌────────────────────┐
   │   STAGE 1       │ │   STAGE 2    │ │    STAGE 3         │
   │  Static         │ │  Sandbox     │ │  Reputation        │
   │  Analysis       │ │  Execution   │ │  Assessment        │
   │                 │ │              │ │                    │
   │ • eval()        │ │ • nsjail     │ │ • Publisher history│
   │ • execSync()    │ │ • 120s limit │ │ • Prior revocations│
   │ • reverse shells│ │ • 512MB RAM  │ │ • Stake level      │
   │ • crypto miners │ │ • net isolate│ │ • Days active      │
   │ • base64 tricks │ │ • Watch:     │ │ • Package count    │
   │ • entropy check │ │   - network  │ │                    │
   │ • typosquatting │ │   - file I/O │ │ Confidence delta:  │
   │   detection     │ │   - processes│ │ +1.0 to -1.0       │
   │                 │ │   - syscalls │ │                    │
   │ Score: 0-100    │ │              │ │ Affects borderline │
   └────────┬────────┘ └──────┬───────┘ └────────┬───────────┘
            │                │                │
            └────────────────┼────────────────┘
                             │
                             ▼
                    COMBINED ASSESSMENT
                             │
                   ┌─────────┴─────────┐
                   │                   │
                   ▼                   ▼
              ValidatorVote       ValidatorVote
              :: Approve          :: Reject { reason }
```

### 3.5 Zero-Knowledge Proofs

After validation, each validator generates a **Groth16 SNARK** — a mathematical proof that:
- They know the package content (without revealing it)
- The content hash matches the claimed hash
- The analysis scores meet the threshold

This proof can be verified by anyone in milliseconds, and cannot be forged.

```
ZK Proof Flow:
──────────────
Validator runs analysis → Gets real scores → Generates Groth16 proof
                                                      │
                    Anyone can verify: ◄──────────────┘
                    • Does this package pass?  YES/NO
                    • Proof size: ~200 bytes
                    • Verify time: < 10ms
                    • Cannot forge without knowing preimage
```

---

## 4. System Architecture Diagram

```
╔══════════════════════════════════════════════════════════════════════════════════╗
║                        CHAIN REGISTRY — FULL ARCHITECTURE                       ║
╚══════════════════════════════════════════════════════════════════════════════════╝

 ╔══════════════════════════════╗
 ║     DEVELOPER WORKSTATION    ║
 ║                              ║
 ║  $ npm install lodash        ║
 ║         │                   ║
 ║         ▼                   ║
 ║  ┌─────────────────┐        ║
 ║  │  creg-npm SHIM  │  PATH  ║
 ║  │  (intercepts)   │  SHIM  ║
 ║  └────────┬────────┘        ║
 ║           │                 ║
 ║           ▼                 ║
 ║  ┌─────────────────┐        ║
 ║  │    RESOLVER     │        ║
 ║  │                 │        ║
 ║  │ ┌─────────────┐ │        ║
 ║  │ │ Local Cache │ │        ║
 ║  │ │ (Sled, TTL) │ │        ║
 ║  │ └──────┬──────┘ │        ║
 ║  │        │ miss   │        ║
 ║  │        ▼        │        ║
 ║  │ ┌─────────────┐ │        ║
 ║  │ │Light Client │ │        ║
 ║  │ │ SPV + Merkle│ │        ║
 ║  │ └─────────────┘ │        ║
 ║  └────────┬────────┘        ║
 ║           │                 ║
 ╚═══════════│═════════════════╝
             │ REST/gRPC
             │
             ▼
╔════════════════════════════════════════════════════════════════════════════════╗
║                         P2P VALIDATOR NETWORK                                  ║
║                                                                                ║
║  ┌──────────────────┐   Gossipsub   ┌──────────────────┐   Gossipsub          ║
║  │   VALIDATOR 1    │◄─────────────►│   VALIDATOR 2    │◄────────────► ...    ║
║  │                  │   (libp2p)    │                  │   (libp2p)           ║
║  │ ┌──────────────┐ │               │ ┌──────────────┐ │                      ║
║  │ │  REST API    │ │               │ │  REST API    │ │                      ║
║  │ │  :8080       │ │               │ │  :8081       │ │                      ║
║  │ │  gRPC :50051 │ │               │ │  gRPC :50052 │ │                      ║
║  │ └──────────────┘ │               │ └──────────────┘ │                      ║
║  │ ┌──────────────┐ │               │ ┌──────────────┐ │                      ║
║  │ │  PENDING     │ │               │ │  PENDING     │ │                      ║
║  │ │  POOL        │ │               │ │  POOL        │ │                      ║
║  │ └──────────────┘ │               │ └──────────────┘ │                      ║
║  │ ┌──────────────┐ │               │ ┌──────────────┐ │                      ║
║  │ │  VALIDATOR   │ │               │ │  VALIDATOR   │ │                      ║
║  │ │  PIPELINE    │ │               │ │  PIPELINE    │ │                      ║
║  │ │  Stage 1:    │ │               │ │  Stage 1:    │ │                      ║
║  │ │   Static     │ │               │ │   Static     │ │                      ║
║  │ │  Stage 2:    │ │               │ │  Stage 2:    │ │                      ║
║  │ │   Sandbox    │ │               │ │   Sandbox    │ │                      ║
║  │ │  Stage 3:    │ │               │ │  Stage 3:    │ │                      ║
║  │ │   Reputation │ │               │ │   Reputation │ │                      ║
║  │ └──────────────┘ │               │ └──────────────┘ │                      ║
║  │ ┌──────────────┐ │               │ ┌──────────────┐ │                      ║
║  │ │ PBFT ENGINE  │ │               │ │ PBFT ENGINE  │ │                      ║
║  │ │ PRE-PREPARE  │ │               │ │ PRE-PREPARE  │ │                      ║
║  │ │ PREPARE      │ │◄─────────────►│ │ PREPARE      │ │                      ║
║  │ │ COMMIT       │ │  vote gossip  │ │ COMMIT       │ │                      ║
║  │ └──────────────┘ │               │ └──────────────┘ │                      ║
║  │ ┌──────────────┐ │               │ ┌──────────────┐ │                      ║
║  │ │ CHAIN STORE  │ │               │ │ CHAIN STORE  │ │                      ║
║  │ │ (Sled DB)    │ │               │ │ (Sled DB)    │ │                      ║
║  │ └──────────────┘ │               │ └──────────────┘ │                      ║
║  └────────┬─────────┘               └────────┬─────────┘                      ║
║           │ Ethereum Bridge                   │ Ethereum Bridge                ║
╚═══════════│═══════════════════════════════════│════════════════════════════════╝
            │                                   │
            └──────────────┬────────────────────┘
                           │ ECDSA quorum signatures
                           ▼
╔══════════════════════════════════════════════════════════════════════════════════╗
║                        ETHEREUM L1 (Anvil / Mainnet)                            ║
║                                                                                  ║
║  ┌────────────┐  ┌────────────┐  ┌──────────────┐  ┌────────────────────────┐  ║
║  │Registry.sol│  │Staking.sol │  │Governance.sol│  │    ZKVerifier.sol      │  ║
║  │            │  │            │  │              │  │                        │  ║
║  │ Package    │  │ Publisher  │  │  M-of-N      │  │  Groth16 SNARK         │  ║
║  │ index      │  │ stakes     │  │  multisig    │  │  on-chain verification │  ║
║  │ Verdicts   │  │ Slashing   │  │  No admin    │  │                        │  ║
║  │ State root │  │ Unbonding  │  │  keys        │  │                        │  ║
║  └────────────┘  └────────────┘  └──────────────┘  └────────────────────────┘  ║
║  ┌────────────┐  ┌────────────┐  ┌──────────────┐  ┌────────────────────────┐  ║
║  │Appeal.sol  │  │VRF.sol     │  │Insurance.sol │  │ CrossChainRegistry.sol │  ║
║  │            │  │            │  │              │  │                        │  ║
║  │ Dispute    │  │ Fair random│  │ Breach       │  │  L2: Arbitrum          │  ║
║  │ resolution │  │ validator  │  │ insurance    │  │  L2: Optimism          │  ║
║  │ Escrow     │  │ selection  │  │ claims       │  │  L2: Polygon           │  ║
║  └────────────┘  └────────────┘  └──────────────┘  └────────────────────────┘  ║
╚══════════════════════════════════════════════════════════════════════════════════╝
                           │
                           ▼
╔══════════════════════════════════════════════╗
║              IPFS NETWORK                    ║
║                                              ║
║  Decentralized tarball storage               ║
║  Content-addressed (CID = SHA-256)           ║
║  Pinned across all validator nodes           ║
║  Resilient to censorship                     ║
╚══════════════════════════════════════════════╝
```

---

## 5. System Flow — Step by Step

### Flow A: Publishing a Package

```
STEP 1 — Developer prepares package
────────────────────────────────────
  Developer runs:
  $ creg publish ./express-4.18.2.tgz

  CLI performs:
  a. Read tarball bytes
  b. Compute SHA-256(tarball) → content_hash
  c. Pin tarball to IPFS → get ipfs_cid
  d. Load Ed25519 private key from ~/.creg/key.pem
  e. Sign: Ed25519(key, canonical || content_hash) → signature
  f. Generate ZK content-hash attestation (Groth16)
  g. Submit PublishRequest via gRPC or REST

  PublishRequest {
    id:               { ecosystem: "npm", name: "express", version: "4.18.2" }
    content_hash:     "sha256:a3f1..."
    ipfs_cid:         "QmXoypizjW3WknFiJnKLwHCnL72vedxjQkDDP1mXWo6uco"
    publisher_pubkey: "hex-encoded-ed25519-pubkey"
    signature:        "hex-encoded-ed25519-signature"
    manifest:         { allowed_network_hosts: [], spawns_processes: false }
    zk_proof:         "serialized-groth16-proof"
  }

STEP 2 — Node receives and validates submission
───────────────────────────────────────────────
  API handler:
  a. Verifies Ed25519 publisher signature
  b. Checks publisher is staked (≥ 1 CREG in Staking.sol)
  c. Checks package not already verified/revoked
  d. Checks for duplicate (same content hash → reject)
  e. Inserts into pending_pool
  f. Broadcasts to all peers via Gossipsub P2P

STEP 3 — Validator pipeline picks it up (every 2 seconds)
──────────────────────────────────────────────────────────
  All validator nodes independently:
  a. Fetch tarball from IPFS using ipfs_cid
  b. Verify SHA-256(tarball) == content_hash  ← hash mismatch = hard reject
  c. Run Stage 1: static_analysis::analyze(tarball)
  d. Run Stage 2: sandbox::run(tarball, manifest) in nsjail
  e. Run Stage 3: reputation::assess(publisher_pubkey)
  f. Combine → ValidatorVote::Approve | Reject

STEP 4 — PBFT Consensus
────────────────────────
  PRE-PREPARE:  VRF selects primary → broadcasts proposal
  PREPARE:      All validators broadcast signed votes
  COMMIT:       Once 2/3+1 votes collected → COMMIT
  FINALIZE:     Write block to Sled DB + broadcast to peers

STEP 5 — Ethereum Bridge
─────────────────────────
  bridge.rs watches chain tip:
  a. Collects 2f+1 ECDSA validator signatures
  b. Calls Registry.sol::finalizePackage(canonical, sigs)
  c. Solidity re-verifies all signatures on-chain
  d. Emits PackageVerified event
  e. Updates latestStateRoot (Merkle root for SPV)
```

### Flow B: Installing a Package

```
STEP 1 — Developer runs install
────────────────────────────────
  $ npm install express
        │
        ▼ (PATH intercept)
  creg-npm shim runs first

STEP 2 — Resolver checks verdict
──────────────────────────────────
  resolver::resolve_id("npm:express@latest")
        │
        ├── Check local Sled cache (TTL: 24h for verified)
        │        │
        │        ├── CACHE HIT  → return cached verdict (fast path, <1ms)
        │        │
        │        └── CACHE MISS → query chain node
        │                              │
        │                    GET /v1/packages/npm:express@latest
        │                              │
        │                    (optional) Verify Merkle proof
        │                    against Registry.sol state root
        │
        └── Return TrustVerdict

STEP 3 — Trust decision
────────────────────────

  VERIFIED   → Install silently (developer sees nothing unusual)
                   │
                   └── (background) Check findings for Critical/High
                         If severe findings: prompt user to confirm

  UNVERIFIED → Warn: "Package not yet chain-verified"
                   │
                   └── --unverified flag: proceed anyway
                       otherwise: bail out

  REVOKED    → HARD BLOCK (exit non-zero, print reason)
               Developer cannot install this package

  UNKNOWN    → Warn: "Not in chain registry"
                   │
                   └── --unverified: fall through to npm

STEP 4 — P2P Download (if verified)
──────────────────────────────────────
  Uses real ipfs_cid from verdict
  Downloads from closest IPFS node / validator gateway
  Verifies content_hash after download
  Falls back to npm registry if P2P fails

STEP 5 — Delegate to real npm
───────────────────────────────
  Calls the real npm (second in PATH)
  Developer experience: identical to normal npm
```

### Flow C: Revoking a Malicious Package

```
STEP 1 — Threat detected
─────────────────────────
  Security researcher finds malicious code in verified package
  OR automated monitoring detects post-publish compromise

STEP 2 — Revocation submitted
───────────────────────────────
  POST /v1/packages/npm:malicious@1.0.0/revoke
  {
    "reason": "Crypto miner in postinstall hook — CWE-506"
  }

STEP 3 — PBFT consensus on revocation
───────────────────────────────────────
  Same quorum required for revocation as for publish
  Prevents single-actor false revocations
  2/3+1 validators must agree

STEP 4 — Chain updated + Ethereum bridge
──────────────────────────────────────────
  Revoke transaction added to block
  Registry.sol::revokePackage() called
  Publisher stake slashed 10% automatically (Staking.sol)
  Slashed tokens distributed to active validators

STEP 5 — All installs blocked immediately
───────────────────────────────────────────
  All resolver caches invalidated via event stream
  Future installs: HARD BLOCK
  Event broadcast: package.revoked (SSE + WebSocket)

STEP 6 — Publisher may appeal
───────────────────────────────
  Publisher calls Appeal.sol::submitAppeal(evidence)
  Requires escrow deposit
  Governance (4-of-7 multisig) reviews
  If approved: reinstatement + escrow returned
  If rejected: escrow forfeited to slash pool
```

---

## 6. Workflow Diagrams

### 6.1 Complete Package Lifecycle

```
                    ┌─────────────┐
                    │  Developer  │
                    │  publishes  │
                    └──────┬──────┘
                           │
                           ▼
                   ┌───────────────┐
                   │ Sign & upload │
                   │ to IPFS       │
                   └──────┬────────┘
                          │
                          ▼
                   ┌───────────────┐
                   │  Submit to    │
                   │  chain node   │◄──── Signature verified?
                   └──────┬────────┘      Publisher staked?
                          │               Duplicate check
                          │ broadcast
                          ▼
               ┌─────────────────────┐
               │    PENDING POOL     │
               │  (all validators)   │
               └──────────┬──────────┘
                          │ poll every 2s
                          ▼
          ┌───────────────────────────────┐
          │      VALIDATION PIPELINE      │
          │                               │
          │  ┌─────────┐ ┌─────────────┐  │
          │  │ Fetch   │ │ Verify hash │  │
          │  │from IPFS│ │ (SHA-256)   │  │
          │  └────┬────┘ └──────┬──────┘  │
          │       │             │         │
          │       ▼             ▼         │
          │  ┌─────────────────────────┐  │
          │  │  Static Analysis        │  │
          │  │  Sandbox Execution      │  │
          │  │  Reputation Assessment  │  │
          │  └──────────┬──────────────┘  │
          │             │                 │
          └─────────────│─────────────────┘
                        │
               ┌────────┴────────┐
               │                 │
               ▼                 ▼
         ✓ Approve            ✗ Reject
               │                 │
               ▼                 ▼
        ┌────────────┐    ┌─────────────┐
        │    PBFT    │    │   Revoke    │
        │  Consensus │    │ Transaction │
        │ (2/3+1     │    │  written    │
        │  quorum)   │    └─────────────┘
        └─────┬──────┘
              │
              ▼
       ┌─────────────┐
       │  New Block  │
       │  written    │
       │  to chain   │
       └──────┬──────┘
              │
     ┌────────┼────────┐
     │        │        │
     ▼        ▼        ▼
  Sled DB  Gossip   Ethereum
  (local)  peers    Bridge
              │        │
              ▼        ▼
         Chain sync  Registry.sol
         to all      finalized
         nodes       on L1
```

### 6.2 PBFT Voting Flow (3 Validators, f=1)

```
Validator A        Validator B        Validator C
(Primary)          (Replica)          (Replica)
    │                   │                   │
    │ VRF selected       │                   │
    │ as primary         │                   │
    │                   │                   │
    │──[PRE-PREPARE]────►│                   │
    │──[PRE-PREPARE]────────────────────────►│
    │                   │                   │
    │                analyze                │
    │                   │               analyze
    │                   │                   │
    │◄──[PREPARE: ✓]────│                   │
    │◄──[PREPARE: ✓]────────────────────────│
    │                   │                   │
    │  2/3+1 votes?     │                   │
    │  2 ≥ 2? YES ✓     │                   │
    │                   │                   │
    │──[COMMIT]─────────►│                   │
    │──[COMMIT]─────────────────────────────►│
    │                   │                   │
    │              write block         write block
    │                   │                   │
    │                broadcast new block    │
    │◄──────────────────│───────────────────│
    │                   │                   │
  Ethereum            Ethereum            Ethereum
  Bridge              Bridge              Bridge
    │                   │                   │
    └───────────────────►───────────────────►
                Registry.sol::finalizePackage()
```

### 6.3 Developer Install Decision Tree

```
$ npm install <package>
       │
       ▼
  creg-npm shim intercepts
       │
       ▼
  resolver::resolve(package)
       │
  ┌────┴────────────────────────────┐
  │                                 │
  ▼                                 ▼
Cache hit?                     Cache miss
  │                                 │
  ▼                          Query chain node
Return                               │
verdict                              ▼
  │                         Verify Merkle proof
  │                         (light-client SPV)
  │                                 │
  │                                 ▼
  │                          Return verdict
  │                                 │
  └────────────────┬────────────────┘
                   │
         ┌─────────┴──────────────────────────────┐
         │          │              │               │
         ▼          ▼              ▼               ▼
     VERIFIED   UNVERIFIED     REVOKED         UNKNOWN
         │          │              │               │
         ▼          ▼              ▼               ▼
    Proceed     Warn user      HARD BLOCK      Warn user
    silently        │          ✗ Exit 1            │
         │      ────┴──────                   ────┴──────
         │      Prompt:       Cannot install   --unverified?
         │      Install?      This package         │
         │          │         is revoked:      YES: proceed
         │      YES: proceed  <reason>         NO: bail
         │      NO: bail
         │
         ▼
    P2P download
    (real IPFS CID)
         │
    ┌────┴─────┐
    │          │
    ▼          ▼
  P2P OK    P2P fail
    │          │
    ▼          ▼
  Use local  Fall back to
  tarball    npm registry
    │          │
    └────┬─────┘
         │
         ▼
   Real npm install
   (second in PATH)
         │
         ▼
   ✓ Done
```

### 6.4 Validator Economic Incentive Flow

```
PUBLISHER                   VALIDATOR                    CHAIN
    │                           │                           │
    │  stake 1 CREG             │                           │
    │─────────────────────────────────────────────────────►│
    │                           │  stake 100 CREG           │
    │                           │──────────────────────────►│
    │                           │                           │
    │  publish package          │                           │
    │──────────────────────────►│ analyze                   │
    │                           │──────────────────────────►│
    │                           │  PBFT consensus           │
    │                           │◄─────────────────────────►│
    │                           │                           │
    │ Package VERIFIED          │                           │
    │◄──────────────────────────│◄──────────────────────────│
    │                           │                           │
    │                           │        [months later]     │
    │                           │                           │
    │  Malicious code found     │                           │
    │─────────────────────────────────────────────────────►│
    │                           │  PBFT revocation          │
    │                           │◄─────────────────────────►│
    │                           │                           │
    │  SLASHED: -10% stake      │                           │
    │◄──────────────────────────────────────────────────────│
    │                           │                           │
    │                           │  Slash pool distributed   │
    │                           │◄──────────────────────────│
    │                           │  (profit from punishment) │
    │                           │                           │
    │  Appeal submitted         │                           │
    │─────────────────────────────────────────────────────►│
    │                           │  Governance vote          │
    │                           │◄─────────────────────────►│
    │                           │                           │
    │  Appeal result            │                           │
    │◄──────────────────────────────────────────────────────│
```

---

## 7. Features

### 7.1 Core Security Features

| Feature | How It Works | Why It Matters |
|---------|-------------|----------------|
| **PBFT Consensus** | 3-phase voting, 2/3+1 quorum | One compromised validator cannot approve malicious packages |
| **Static Code Analysis** | 8+ malicious patterns, Shannon entropy, typosquatting | Catches obfuscated miners, reverse shells, eval tricks |
| **Sandbox Execution** | nsjail: 120s limit, 512MB RAM, network isolated | Dynamic analysis — code cannot hide behavior when run |
| **Reputation Scoring** | Publisher history on-chain: revocations, stake, age | Publishers with bad track records face heightened scrutiny |
| **ZK-SNARK Proofs** | Groth16 on BN254 curve | Cryptographic, unforgeable validation attestations |
| **ML Threat Detection** | ONNX runtime + feature extraction | AI-based detection of novel malware patterns |
| **PGP Web-of-Trust** | GPG signature verification | Confirms package identity through key signing networks |
| **Differential Analysis** | Diff vs previous version | Catches malicious additions in version updates |
| **Economic Slashing** | 10% stake slashed on revocation | Financial punishment for publishing malicious code |
| **Ethereum Anchoring** | L1 finality for all verdicts | Permanent, tamper-proof audit trail of all decisions |

### 7.2 Developer Experience Features

| Feature | Description |
|---------|-------------|
| **Zero workflow change** | npm/pip/cargo/gem/mvn work exactly as before |
| **PATH shims** | Transparent intercept via `creg setup-shims` |
| **Local verdict cache** | Sled TTL cache — verified packages: 24h cache |
| **Light-client SPV** | Verify verdicts without running a full node |
| **Lockfile support** | `pkg-lock.chain` — reproducible, auditable installs |
| **Audit command** | `creg audit` — scan all installed packages |
| **Watch command** | `creg watch` — real-time block explorer in terminal |
| **Batch operations** | `creg batch` — verify multiple packages at once |

### 7.3 Governance Features

| Feature | Description |
|---------|-------------|
| **M-of-N Multisig** | No single admin — all parameter changes need 4-of-7 approval |
| **No admin keys** | System governed by validator quorum, not a company |
| **Time-locked changes** | GovernanceV2 prevents rapid parameter manipulation |
| **Appeal mechanism** | Publishers can contest revocations with evidence + escrow |
| **CREG Token** | ERC-20 for DAO governance voting on protocol changes |
| **Emergency pause** | Requires governance quorum — cannot be done unilaterally |

### 7.4 Infrastructure Features

| Feature | Description |
|---------|-------------|
| **IPFS storage** | Decentralized tarball hosting — no single CDN to attack |
| **libp2p P2P** | Gossipsub + Kademlia — resilient, self-healing network |
| **Prometheus metrics** | `/metrics` endpoint — 15+ tracked metrics |
| **Grafana dashboards** | 10 pre-built panels, 7 alerting rules |
| **Docker Compose** | Full 3-node cluster with one command |
| **SSE event stream** | Real-time events for UIs, monitoring, automation |
| **WebSocket support** | Low-latency event streaming |
| **gRPC API** | High-performance binary protocol (port 50051) |
| **L2 support** | State roots on Arbitrum, Optimism, Polygon |
| **Package insurance** | On-chain breach insurance for affected users |

---

## 8. How People Use It

### 8.1 For Individual Developers

```bash
# One-time setup (takes 2 minutes)
cargo install creg
creg keygen                              # Generate your keypair
creg setup-shims                         # Install PATH shims

# After setup — everything works exactly the same:
npm install express                      # ← intercepted silently
pip install requests                     # ← intercepted silently
cargo add tokio                          # ← intercepted silently
gem install rails                        # ← intercepted silently

# Check a package before installing
creg status npm:express@4.18.2

# Audit all your installed packages
creg audit

# Install with override (for testing)
creg install lodash --unverified
```

### 8.2 For Package Publishers

```bash
# Generate your publisher keypair (one time)
creg keygen --output ~/.creg/publisher.pem

# Stake to publish (minimum 1 CREG — refundable deposit)
creg stake --amount 1creg --key ~/.creg/publisher.pem

# Publish a package
creg publish ./my-library-1.0.0.tgz \
  --key ~/.creg/publisher.pem \
  --manifest ./creg.manifest.json

# Optional: publish a shielded (encrypted) package
creg publish ./my-library-1.0.0.tgz --shield

# Monitor your packages
creg watch                                # Live block explorer
creg status cargo:my-library@1.0.0       # Check specific version
```

**creg.manifest.json** — tell validators what your package is allowed to do:
```json
{
  "allowed_network_hosts": [],
  "allowed_fs_writes": ["/tmp"],
  "spawns_processes": false,
  "description": "A pure-function math library with no side effects"
}
```

### 8.3 For Security Teams & Organizations

```bash
# Run your own private validator node
docker run -e CREG_IS_VALIDATOR=true \
           -e CREG_VALIDATOR_KEY=<hex-privkey> \
           -e CREG_VALIDATOR_SET=<json> \
           chain-registry-node

# Set up a full local cluster
docker-compose up -d                     # 3 validators + IPFS + Anvil

# Configure company-wide policy via environment
CREG_ALLOW_UNVERIFIED=false             # Block all unverified packages
CREG_NODE_URL=https://registry.mycompany.com

# Audit pipeline integration (CI/CD)
creg audit --format json | jq '.[] | select(.status != "verified")'
```

**creg.policy.json** — enforce across the organization:
```json
{
  "block_unverified": true,
  "block_unknown": true,
  "min_validator_count": 7,
  "require_pgp_signature": true
}
```

### 8.4 For Validator Node Operators

```bash
# Requirements:
# - 100 CREG stake (unbonding: 7 days) — governance must approve your application first
# - Linux server (for nsjail sandbox support)
# - 8GB RAM minimum, SSD storage
# - Stable internet connection

# Stake as validator
creg stake validator --amount 1eth --key ~/.creg/validator.pem

# Start validator node
CREG_IS_VALIDATOR=true \
CREG_VALIDATOR_KEY=$(cat ~/.creg/validator.pem) \
CREG_PEERS=http://node1.registry.io,http://node2.registry.io \
creg-node

# Monitor your validator
creg watch                              # Block explorer
curl localhost:8080/metrics             # Prometheus metrics
curl localhost:8080/v1/chain/stats      # Chain statistics
```

### 8.5 REST API Reference

```bash
# Health check
GET  /v1/health

# Package operations
GET  /v1/packages/{canonical}              # Get verdict
POST /v1/packages                          # Submit for validation
POST /v1/packages/{canonical}/revoke       # Revoke a package
GET  /v1/packages/{canonical}/proof        # Get SPV Merkle proof

# Chain data
GET  /v1/chain/stats                       # Tip height, package count
GET  /v1/blocks/{height}                   # Block by height
GET  /v1/blocks/hash/{hash}                # Block by hash

# Network
GET  /v1/nodes                             # Active validator set
GET  /v1/p2p/status                        # P2P peer connections
GET  /v1/bridge/status                     # Ethereum bridge status

# Events (real-time)
GET  /v1/events                            # SSE event stream
GET  /v1/ws                               # WebSocket events

# Consensus (validator-only)
POST /v1/consensus/vote                    # Submit authenticated vote

# Observability
GET  /metrics                              # Prometheus metrics
```

### 8.6 Event Stream Integration

```javascript
// Browser / Node.js — subscribe to real-time events
const events = new EventSource('http://localhost:8080/v1/events');

events.addEventListener('package_verified', (e) => {
  const data = JSON.parse(e.data);
  console.log(`✓ ${data.canonical} verified in block ${data.block_hash}`);
});

events.addEventListener('package_revoked', (e) => {
  const data = JSON.parse(e.data);
  console.log(`✗ ${data.canonical} REVOKED: ${data.reason}`);
  triggerAlert(data);
});

events.addEventListener('block_produced', (e) => {
  const data = JSON.parse(e.data);
  updateDashboard(data.height, data.tx_count);
});
```

---

## 9. Why It Matters — The Importance

### 9.1 The Scale of the Problem

```
Software Dependency Landscape (2024):
──────────────────────────────────────

  npm alone:          2.1 million packages
  PyPI:               500,000+ packages
  crates.io:          150,000+ packages
  Average app:        500–1,500 dependencies (including transitive)

  A developer installing a React app today
  implicitly trusts:  ~1,000 strangers' code to run on their machine.

  Zero of those strangers have been independently verified.
```

### 9.2 Why Existing Solutions Fail

| Solution | Why It's Insufficient |
|----------|----------------------|
| **npm audit** | Checks for known CVEs only. Does not detect new malware. |
| **Package signing (npm provenance)** | Proves the package came from the claimed repo. Does NOT analyze what the code does. |
| **Snyk / Dependabot** | Database-driven. Cannot detect zero-day supply chain attacks. |
| **Manual code review** | Does not scale to 1,000+ transitive dependencies. |
| **Sandboxed install** | Rare. Not default behavior. Requires developer action. |

Chain Registry adds what none of these have: **decentralized, consensus-based, economically-incentivized, automated multi-stage analysis** that happens before any developer can install a package.

### 9.3 The Defense-in-Depth Advantage

```
LAYERED SECURITY MODEL:
───────────────────────────────────────────────────────────────────

Layer 1: Static Analysis
  "Does this code contain known malicious patterns?"
  Catches: eval(), execSync(), crypto miners, reverse shells
  Speed: < 1 second

Layer 2: Sandbox Execution
  "What does this code actually DO when it runs?"
  Catches: Hidden network calls, file exfiltration, process spawning
  Speed: 30–120 seconds

Layer 3: Reputation Assessment
  "Who wrote this, and have they been trustworthy before?"
  Catches: Pattern of increasingly malicious packages from same publisher
  Speed: < 1 second

Layer 4: ML Threat Detection
  "Does this code exhibit novel malicious behavior patterns?"
  Catches: Obfuscation patterns invisible to static analysis
  Speed: 1–5 seconds

Layer 5: ZK Proof
  "Can we prove this analysis happened correctly, cryptographically?"
  Purpose: Unforgeable attestation for auditors and light clients
  Speed: 100–500ms verification

Layer 6: PBFT Consensus
  "Do at least 2/3 of independent validators agree?"
  Catches: Single compromised validator approving malicious code
  Speed: 5–30 seconds (network round-trips)

Layer 7: Economic Slashing
  "Is there a financial cost for approving malicious code?"
  Catches: Negligent or colluding validators (they lose 10% stake)
  Purpose: Long-term behavioral alignment of validators

Layer 8: Ethereum Anchoring
  "Is this verdict permanently recorded and publicly verifiable?"
  Purpose: Audit trail, historical accountability, legal evidence
  Speed: 12 seconds (L1 finality)

──────────────────────────────────────────────────────────────────
An attacker must defeat ALL 8 layers simultaneously.
This has never been done.
```

### 9.4 Economic Security Model

```
STAKE → ACCOUNTABILITY → SECURITY:

If a validator approves a malicious package:
  → Package gets revoked when discovered
  → Validator loses 10% of their 100 CREG stake = 10 CREG penalty
  → Slashed CREG redistributed to honest validators (proportional to reputation score)
  → Validator reputation drops
  → After 3 slashes: validator is automatically ejected (unbonding begins)
  → If repeated: validator eventually forced out

If a publisher submits malicious code:
  → Package gets revoked
  → Publisher loses 10% of stake
  → Publisher reputation destroyed on-chain (permanent)
  → Cannot resubmit revoked package

The math for an attacker:
  To bribe 1/3 of 10 validators (4 validators) to approve bad code:
  4 × 100 CREG minimum bribe + risk of slash + risk of exposure
  + must bypass the governance approval gate (cannot become a validator instantly)
  vs. benefit: distributing one malicious npm package

  At scale (1000 validators, institutional operators):
  Attack cost: 300 ETH+ (prohibitively expensive)
```

### 9.5 What This Enables

```
With Chain Registry, for the first time:

  ✓ Developers know BEFORE installing that code is safe
  ✓ Security decisions are made by consensus, not one company
  ✓ Malicious publishers have skin in the game (real financial cost)
  ✓ All decisions are permanently auditable on Ethereum
  ✓ Anyone can run a validator and participate in governance
  ✓ Packages can be insured against breach
  ✓ No single point of failure or censorship
  ✓ Works transparently with existing tooling — zero re-training
```

---

## 10. Comparison: Before vs After

### The Developer Experience

| Scenario | Without Chain Registry | With Chain Registry |
|----------|----------------------|---------------------|
| Install safe package | `npm install express` ✓ | `npm install express` ✓ (same) |
| Install malicious package | `npm install event-stream` 💀 | BLOCKED with reason |
| Install typosquatted package | `npm install lodahs` 💀 | BLOCKED (typosquat detected) |
| Install from compromised registry | `npm install axios` 💀 | BLOCKED (hash mismatch) |
| Check if package is safe | Manual audit, hope for the best | `creg status npm:axios@1.6.0` |
| Audit all dependencies | Run 5 different tools, incomplete | `creg audit` — complete chain scan |

### The Security Posture

```
WITHOUT Chain Registry:              WITH Chain Registry:
──────────────────────────           ──────────────────────────────────

Trust: npm says so                   Trust: 2/3+ independent validators
Verification: none (BYOK)            Verification: static + sandbox + AI
Response to threat: hours/days       Response to threat: minutes
Accountability: none                 Accountability: staked + slashed
Auditability: npm logs (private)     Auditability: public blockchain
Decentralization: one company        Decentralization: 10–1000 validators
Censorship resistance: zero          Censorship resistance: BFT
```

### The Trust Model

```
OLD MODEL:                           NEW MODEL:

  You → trust → npm                  You → verify → Chain Registry
                                         → verify → Ethereum L1
  (single point of failure)               → verify → Merkle proof
                                              │
                                         Chain Registry →
                                           trust (with proof) →
                                           10+ validators →
                                           each independently →
                                           analyzed the code
```

---

## Glossary

| Term | Definition |
|------|-----------|
| **BFT** | Byzantine Fault Tolerant — system that works correctly even if some nodes are malicious |
| **PBFT** | Practical BFT — the specific consensus protocol used (3-phase: PRE-PREPARE, PREPARE, COMMIT) |
| **Quorum** | Minimum votes needed: ⌊2n/3⌋ + 1 where n = validator count |
| **Groth16** | A type of ZK-SNARK proof system — small proofs, fast verification |
| **SNARK** | Succinct Non-interactive ARgument of Knowledge — a type of zero-knowledge proof |
| **SPV** | Simplified Payment Verification — verifying a single record using a Merkle proof without downloading the whole chain |
| **Merkle Root** | A single hash summarizing all transactions in a block — used for light-client verification |
| **Slashing** | Automatically removing a percentage of a validator's stake as punishment for bad behavior |
| **Unbonding** | The 7-day waiting period after a validator leaves before they can withdraw their stake |
| **Ed25519** | An elliptic curve signature scheme used for all signing operations |
| **IPFS CID** | Content Identifier — a hash of content used to locate it in IPFS |
| **Canonical** | The unique string identifier for a package: `ecosystem:name@version` (e.g., `npm:express@4.18.2`) |
| **nsjail** | A Linux kernel-level sandboxing tool that isolates processes using namespaces |
| **Gossipsub** | A pub/sub protocol in libp2p used to broadcast votes and blocks across the P2P network |
| **Kademlia DHT** | A distributed hash table for peer discovery in the P2P network |

---

*Chain Registry — Securing the global software supply chain, one block at a time.*

*Document version: 2026-03-31*
