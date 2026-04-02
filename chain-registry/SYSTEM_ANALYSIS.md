# Chain Registry: Architecture Overview

The **Chain Registry** is a decentralized, consensus-driven package distribution system designed to secure software supply chains.

## 🏗️ Core Architecture
The system consists of a P2P network of validation nodes (AppChain) that perform behavioral analysis on packages before they are finalized. All trust verdicts are recorded on a persistent blockchain and bridged to Ethereum L1 for global finality.

### 🧩 Key Components
- **Consensus**: PBFT (2/3 majority requirement).
- **Storage**: IPFS (Decentralized tarballs) + Sled (On-node ledger).
- **Communication**: libp2p Gossipsub (Submissions/Votes).
- **Enforcement**: 3-Stage Validator Pipeline (Static, Sandbox, ML).

---

## 🚀 System Lifecycle

### 1. Publishing
Developers use `creg publish` to sign and upload packages. Submissions are gossiped across the network for verification.

### 2. Validation & Consensus
Nodes execute packages in isolated `nsjail` sandboxes. Findings are analyzed, and a PBFT quorum reaches a verdict (Verified vs. Rejected).

### 3. Installation
Developer machines use **PATH Shims** to intercept calls to `npm`, `pip`, or `cargo`. These shims verify the package against the blockchain before allowing execution.

---

## 🔎 Deep-Dive Documentation

For a full technical analysis including diagrams, tokenomics, and crate-level details, see:

> [!IMPORTANT]
> **[Chain_Registry_Technical_Report.md](file:///C:/Users/samue/.gemini/antigravity/brain/586bfe0a-70ef-4c8f-a7a7-233f2253b06c/Chain_Registry_Technical_Report.md)**

---

## 📅 Roadmap & Progress
- [x] P2P Gossip Layer (Hardened)
- [x] Ethereum L1 Bridge (Operational)
- [x] Security Sandbox (nsjail Integrated)
- [x] CLI Dashboards (Live TUI)
