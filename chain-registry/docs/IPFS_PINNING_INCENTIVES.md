# IPFS Pinning Incentives System

## Overview

The IPFS Pinning Incentives system provides economic rewards to nodes that store and serve package content. This ensures content persistence and availability across the Chain Registry network.

## Why This Matters

Without incentives, IPFS content relies on altruistic pinning. If the original publisher stops their node, content can disappear. The pinning rewards system solves this by:

1. **Paying nodes to store content** - Economic motivation for long-term storage
2. **Verifying actual storage** - Random checks ensure nodes aren't cheating
3. **Rewarding popular content** - More rewards for frequently accessed packages
4. **Punishing failures** - Slashing for nodes that claim to pin but don't

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                    IPFS PINNING INCENTIVES                       │
├─────────────────────────────────────────────────────────────────┤
│                                                                  │
│  ┌─────────────┐      ┌─────────────┐      ┌─────────────┐     │
│  │   Package   │      │    IPFS     │      │   Mirror    │     │
│  │  Verified   │─────▶│   Network   │◄─────│    Node     │     │
│  │   by Chain  │      │             │      │  (Pinner)   │     │
│  └─────────────┘      └─────────────┘      └──────┬──────┘     │
│                                                    │            │
│                                                    ▼            │
│  ┌──────────────────────────────────────────────────────────┐  │
│  │              PinningRewards.sol (Ethereum)                │  │
│  │  ┌─────────────┐  ┌─────────────┐  ┌─────────────────┐  │  │
│  │  │   Pinner    │  │    Pin      │  │  Verification   │  │  │
│  │  │  Registry   │  │   Tracking  │  │     Records     │  │  │
│  │  └─────────────┘  └─────────────┘  └─────────────────┘  │  │
│  │                                                           │  │
│  │  Rewards = Size × Time × Rate × Reliability × Popularity  │  │
│  └──────────────────────────────────────────────────────────┘  │
│                              │                                   │
│                              ▼                                   │
│  ┌──────────────────────────────────────────────────────────┐  │
│  │              ipfs-pinner Crate (Rust)                     │  │
│  │  ┌─────────────┐  ┌─────────────┐  ┌─────────────────┐  │  │
│  │  │   IPFS      │  │   Contract  │  │   Verification  │  │  │
│  │  │   Client    │  │   Interface │  │    Service      │  │  │
│  │  └─────────────┘  └─────────────┘  └─────────────────┘  │  │
│  └──────────────────────────────────────────────────────────┘  │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

## Smart Contract: PinningRewards.sol

### Key Features

#### 1. Pinner Registration
```solidity
function registerPinner(uint256 stakeAmount) external
```
- Minimum stake: **1000 CREG**
- Stake locked while pinning
- Slashed for failed verifications

#### 2. Pin Registration
```solidity
function registerPin(bytes32 cid, uint256 size) external
```
- Register CID being pinned
- Track content size for rewards
- Multiple pinners per CID (redundancy)

#### 3. Verification System
```solidity
function submitVerification(
    address pinner,
    bytes32 cid,
    bool success,
    bytes32 proofHash
) external
```
- Random sampling of pinned content
- Proof-of-storage via DHT lookups
- 1% stake slashed per failed verification

#### 4. Rewards Calculation
```solidity
function calculateRewards(address pinner) public view returns (uint256)
```

**Formula:**
```
Reward = Size(GB) × Days × BaseRate × Reliability × Popularity

Where:
- Size(GB): Content size in gigabytes
- Days: Time pinned
- BaseRate: 0.01 CREG per GB per day
- Reliability: Success rate % (0-100%)
- Popularity: 2x bonus if >1000 accesses
```

### Economic Parameters

| Parameter | Value | Description |
|-----------|-------|-------------|
| Min Stake | 1000 CREG | Entry barrier |
| Base Rate | 0.01 CREG/GB/day | Base reward rate |
| Popular Bonus | 2x | For content >1000 accesses |
| Slash Penalty | 1% | Per failed verification |
| Verify Cooldown | 1 hour | Prevent spam |

## Rust Implementation: ipfs-pinner Crate

### Components

#### 1. PinningManager
Main coordinator that:
- Registers pins on-chain
- Tracks local pinning state
- Runs verification loops
- Claims rewards

```rust
use ipfs_pinner::{PinningManager, PinningConfig};

let config = PinningConfig {
    ipfs_url: "http://localhost:5001".to_string(),
    eth_rpc: "http://localhost:8545".to_string(),
    contract_address: "0x...".to_string(),
    operator_key: "0x...".to_string(),
    auto_register: true,
    verification_interval: 3600, // 1 hour
};

let manager = PinningManager::new(config).await?;
manager.start().await?;
```

#### 2. IpfsPinner
Interface to IPFS node:
- Pin/unpin CIDs
- Check pinning status
- Fetch content
- Get repo stats

#### 3. Verifier
Verifies content availability:
- DHT provider lookups
- Lightweight content checks
- Generates proof hashes
- Batch verification

## Usage Examples

### 1. Start a Mirror Node

```bash
# 1. Stake CREG to register as pinner
creg pinner register --stake 1000

# 2. Start the pinning service
creg pinner start \
  --ipfs-url http://localhost:5001 \
  --eth-rpc http://localhost:8545 \
  --contract 0x...

# 3. Automatic operations:
#    - Pins new verified packages
#    - Runs verifications hourly
#    - Claims rewards daily
```

### 2. Check Pinning Status

```bash
# List pinned CIDs
creg pinner list

# Show statistics
creg pinner stats
# Output:
# Total pins: 1,234
# Total size: 45.6 GB
# Monthly rewards: 13.68 CREG
# Verification rate: 99.2%

# Check pending rewards
creg pinner rewards
```

## Reward Calculations

### Example 1: Small Package
```
Package: npm:lodash@4.17.21
Size: 1.5 MB = 0.0015 GB
Pinned for: 30 days
Accesses: 50
Verification rate: 100%

Reward = 0.0015 × 30 × 0.01 × 1.0 × 1.0
       = 0.00045 CREG (0.45 milliCREG)
```

### Example 2: Popular Package
```
Package: npm:react@18.2.0
Size: 15 MB = 0.015 GB
Pinned for: 30 days
Accesses: 5,000 (>1000, gets bonus)
Verification rate: 100%

Reward = 0.015 × 30 × 0.01 × 1.0 × 2.0
       = 0.009 CREG (9 milliCREG)
```

### Example 3: Node with 1000 Packages
```
Average size: 5 MB = 0.005 GB
Total: 1000 packages = 5 GB
Pinned for: 30 days
Average verification: 98%
Mixed popularity (30% popular)

Base reward = 5 × 30 × 0.01 = 1.5 CREG
Reliability = 0.98
Popularity bonus = ~1.3x average

Total = 1.5 × 0.98 × 1.3 ≈ 1.91 CREG/month
```

## API Reference

### Smart Contract

```solidity
// Read functions
function getPinnerInfo(address pinner) external view returns (Pinner memory);
function getPinInfo(bytes32 cid) external view returns (Pin memory);
function calculateRewards(address pinner) external view returns (uint256);

// Write functions
function registerPinner(uint256 stake) external;
function registerPin(bytes32 cid, uint256 size) external;
function claimRewards() external;
```

### Rust Library

```rust
// Main types
pub struct PinningManager;
pub trait IpfsPinner;
pub trait PinningContract;
pub trait Verifier;

// Key methods
impl PinningManager {
    pub async fn pin_package(&self, cid: &str, size: u64) -> Result<()>;
    pub async fn get_stats(&self) -> PinnerStats;
    pub async fn claim_rewards(&self) -> Result<u128>;
}
```

## Resources

- **Contract**: `contracts/PinningRewards.sol`
- **Rust Crate**: `crates/ipfs-pinner/`
- **Tests**: `cargo test -p ipfs-pinner`

---

*Last updated: 2026-04-02*
