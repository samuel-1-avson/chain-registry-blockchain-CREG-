# Chain Registry (CREG) — Documentation Index & Knowledge Base

Welcome to the canonical documentation for **Chain Registry (CREG)**, an evidence-backed, multi-validator decentralized package registry anchored to Ethereum Sepolia.

---

## 1. Core Architecture & Workflows (Start Here)

These documents explain how the system functions conceptually, architecturally, and operationally:

| Document | Description | Target Audience |
|:---|:---|:---|
| **[CREG-COMPLETE-DOCUMENTATION.md](./CREG-COMPLETE-DOCUMENTATION.md)** | **Complete Documentation (start here):** system architecture, workflow, blockchain structure, consensus, validator architecture, tokenomics, publisher / validator / consumer flows, security model, status and roadmap. Every capability is tagged Live, Built, Design or Retired. | Everyone |
| **[CREG-DEEP-DIVE-AUDIT-2026-11-14.md](./CREG-DEEP-DIVE-AUDIT-2026-11-14.md)** | **Current audit:** live state, findings register (AUD-01…27), ratings and the recommended implementation route. | Maintainers, Auditors |
| **[USER_AND_SYSTEM_WORKFLOW_GUIDE.md](./USER_AND_SYSTEM_WORKFLOW_GUIDE.md)** | **Workflow Guide (predates R7; see its status notice):** walkthroughs for running a validator node and publishing packages. | Developers, Operators, Stakeholders |
| **[REMOTE_INSTALLATION_GUIDE.md](./REMOTE_INSTALLATION_GUIDE.md)** | **Universal Remote Installation:** 1-line curl/powershell scripts, precompiled binaries, and Docker shims for external machines. | All External Users & Operators |
| **[DEEP_SYSTEM_ANALYSIS_AND_PROGRESS_REPORT.md](./archive/DEEP_SYSTEM_ANALYSIS_AND_PROGRESS_REPORT.md)** (archived 2026-09-19) | **System Audit & Scorecard (Rating: 9.7/10):** Full architectural review, pros/cons, rating matrix, and subsystem deep dives. | Architects, Security Auditors |
| **[WHITEPAPER.md](./WHITEPAPER.md)** | **Foundational Whitepaper:** Economic model, dual-consensus thesis, and cryptographic supply-chain security proofs. | Researchers, Engineers, DAO Members |
| **[CREG_SYSTEM_WORKFLOW.md](./CREG_SYSTEM_WORKFLOW.md)** | **System Specification:** Original formal specification of CREG's end-to-end verification workflows. | Core Protocol Developers |
| **[FULL_DECENTRALIZATION_ROADMAP.md](./FULL_DECENTRALIZATION_ROADMAP.md)** | **Decentralization Roadmap:** Transition strategy from genesis bootstrap to community-governed multi-validator mesh. | Community, Operators |
| **[DUAL_KEY_ARCHITECTURE_ENHANCEMENT_PROPOSAL.md](./archive/DUAL_KEY_ARCHITECTURE_ENHANCEMENT_PROPOSAL.md)** (archived 2026-09-19) | **Dual-Key UX Enhancement Proposal:** 5-pillar strategy for deterministic key derivation, `creg login`, EIP-712 binding, and Passkeys. | Core Protocol Developers, UX Architects |

---

## 2. Infrastructure & System Operations

Documents detailing the live cloud infrastructure, RPC routing, and high availability systems:

| Document | Description | Live Endpoints |
|:---|:---|:---|
| **[RPC_ARCHITECTURE_AND_SYSTEM_REPORT.md](./archive/RPC_ARCHITECTURE_AND_SYSTEM_REPORT.md)** (archived 2026-09-19) | Unified Edge Gateway (Caddy v2), JSON-RPC engine, and public TLS ingress. | `https://api.testnet.cregnet.dev` |
| **[UNINTERRUPTIBLE_OBSERVER_NODE_SYSTEM.md](./UNINTERRUPTIBLE_OBSERVER_NODE_SYSTEM.md)** | Dedicated Observer VM in Zone A with automatic edge failover and zero-downtime routing. | Zone `us-central1-a` (`10.128.0.41:28182`) |
| **[GCP-RPC-ARCHITECTURE.md](./archive/GCP-RPC-ARCHITECTURE.md)** (archived 2026-09-19) | Cloud cost optimization (~$59/mo) and single-instance ingress consolidation model. | VM `creg-testnet-vm` (`35.225.225.20`) |
| **[DATABASE_SCHEMA.md](./DATABASE_SCHEMA.md)** | Persistent RocksDB state storage, key prefixes, and indexing schema. | `/data` volume storage |

---

## 3. Operator Onboarding & Runbooks

Step-by-step guides for external community operators to run nodes, stake tokens, and witness the log:

| Document | Role | Primary Commands |
|:---|:---|:---|
| **[VALIDATOR_ONBOARDING_CHECKLIST.md](./VALIDATOR_ONBOARDING_CHECKLIST.md)** | **Validator Operators:** Prerequisites, hardware sizing, 100 tCREG staking, and dual-key registration. | `creg stake`, `./testnet/start-external-validator.sh` |
| **[PUBLIC_TESTNET_QUICKSTART.md](./PUBLIC_TESTNET_QUICKSTART.md)** | **Package Developers:** Publishing packages, testing the CLI, and checking the explorer. | `creg publish`, `creg install` |
| **[WITNESS_OPERATOR_ONBOARDING.md](./WITNESS_OPERATOR_ONBOARDING.md)** | **Witness Operators:** Independent tree head consistency checking to prevent split-view attacks. | `creg witness run` |
| **[WITNESS_KEY_CEREMONY.md](./WITNESS_KEY_CEREMONY.md)** | **Key Ceremony:** Cryptographic key generation standards for log signers and witnesses. | Ed25519 ceremony protocols |
| **[WALLET_KEY_DERIVATION.md](./WALLET_KEY_DERIVATION.md)** | **Key Standards:** Differences between secp256k1 EVM wallets and Ed25519 consensus keys. | Key separation standards |

---

## 4. Security, Hardening & Incident Response

Standard operating procedures for maintaining network resilience and responding to anomalies:

| Document | Description | Scope |
|:---|:---|:---|
| **[INCIDENT_RESPONSE_RUNBOOK.md](./INCIDENT_RESPONSE_RUNBOOK.md)** | Emergency procedures for chain forks, split views, consensus halts, and malicious package outbreaks. | Protocol Incidents |
| **[SECURITY_OPS_RUNBOOK.md](./SECURITY_OPS_RUNBOOK.md)** | Key management hygiene, Secret Manager integration, firewall policies, and TLS certificate rotation. | Operator Security |
| **[OPS_HARDENING_RUNBOOK.md](./OPS_HARDENING_RUNBOOK.md)** | Host system tuning, container resource limits, and network interface isolation. | Production Node Hardening |
| **[PROTOCOL_PROGRESS.md](./PROTOCOL_PROGRESS.md)** | Living protocol tracker tracking shipped capabilities vs active testnet milestones. | Protocol Progress |

---

## 5. Specialized Subdirectories

- **[adr/](./adr/)**: Architectural Decision Records (e.g., ADR on KMS Hot Keys, consensus parameters).
- **[architecture/](./architecture/)**: Detailed component diagrams and subsystem specifications.
- **[governance/](./governance/)**: 2-of-3 Multi-Sig governance proposal standards and parameter update runbooks.
- **[operations/](./operations/)**: Day-to-day maintenance and monitoring procedures.
- **[protocol/](./protocol/)**: Formal protocol rules, message formats (`creg-vote-v2`, `creg-pbft-v1`), and namespace authorities.
- **[security/](./security/)**: Threat models, sandbox escape mitigation, and scanner profiles.
- **[testing/](./testing/)**: Acceptance gates, soak testing frameworks, and adversarial test suites.

---

## 6. Historical Document Archive ([docs/archive/](./archive/))

To keep this directory clean and maintainable, all **completed milestone reports, historical research drafts, initial implementation plans, and retired lab documentation** have been preserved in:

📁 **[docs/archive/](./archive/)**

Key archived documents include:
- Completed milestone completion reports (`LAYER1_ENHANCEMENTS_COMPLETION_REPORT.md`, `SUBMISSION_LAYER_PHASE1_COMPLETION_REPORT.md`, etc.).
- Historical testnet and simulation reports (`MULTI_NODE_TESTNET_SIMULATION_REPORT.md`, `TESTNET_READINESS_REPORT.md`).
- Past audits and outreach drafts (`CREG_COMPREHENSIVE_AUDIT_2026-07-10.md`, `SEC-401-AUDIT-SCOPE.md`).
- Retired lab runbooks (`GCP-VALIDATOR-FLEET.md`, `HYBRID-LOCAL-VALIDATORS.md`, `WAITLIST-ONLY-HOSTING.md`).
- Genesis Wave 1 outreach materials (`GENESIS_ALPHA_COHORT_PLAYBOOK.md`, `genesis-alpha-wave1-tracker.csv`).
- Former live references archived 2026-09-19 (`TESTNET_STATUS.md`, `TESTNET_PHASE_SCOPE.md`, `WHAT_TO_DO_NEXT.md`, `NEXT_WORK.md`, `PUBLIC.md`, `TESTNET_SCALE_ARCHITECTURE.md`); current protocol briefing is [`PROTOCOL_PROGRESS.md`](./PROTOCOL_PROGRESS.md).
