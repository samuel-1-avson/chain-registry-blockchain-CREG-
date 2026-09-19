# Full Decentralization Roadmap & Architecture Implementation Plan

This plan details every centralized component currently in the Chain Registry (CREG) ecosystem and establishes the concrete engineering steps to transition each component to **complete, autonomous decentralization**.

---

## 1. Decentralization Audit: What is Currently Centralized?

Our audit of the codebase, cloud deployment, and testnet topology identified **6 major centralization bottlenecks**:

| # | Component | Current Centralized State | Decentralized Target State |
|---|:---|:---|:---|
| **1** | **Observer Node Hosting** | The observer node (`creg-unified-node`) is co-located on a single GCP VM (`creg-testnet-vm`) sharing resources with Caddy, Faucet, Explorer, and IPFS. | **Independent Observer Host / Multi-Zone Cluster:** Observer nodes run on dedicated VMs, decoupled from public web/edge hosting, resilient to single-host outages. |
| **2** | **Genesis Validator Operators** | All 4 active genesis validators (`validator-1`...`validator-4`) are operated locally or in a coordinated maintainer environment using pre-seeded test keys. Zero external operators. | **Autonomous Validator Network:** Multi-party operators across distinct physical hardware, geographic regions, and independent cloud accounts. |
| **3** | **P2P Peer Discovery (Bootnodes)** | `chain-spec.json` has `"bootnodes": []`. External nodes have no public rendezvous points or multiaddresses to discover peers. | **Public Seed / Bootnode Network:** Globally reachable bootnodes with published libp2p multiaddresses configured in `chain-spec.json`. |
| **4** | **Validator Admission Flow** | Nodes are statically hardcoded in genesis or manually triggered by the project maintainer. | **Permissionless Mechanical Admission:** Operators stake $\ge 100\text{ tCREG}$ on Sepolia `Staking.sol`, submit an EIP-712 registration, and get admitted via $\lfloor 2n/3 \rfloor + 1$ validator consensus (`approveByConsensus`). |
| **5** | **Transparency Witness Network** | `testnet/witnesses/roster.json` is unpopulated. No independent third-party witnesses monitor the Merkle Tree Head for split-view attacks. | **Distributed Witness Roster:** At least 2–3 independent external witness daemons (`creg witness run`) continuously attesting and cross-signing the tree head. |
| **6** | **Smart Contract Administration** | Sepolia L1 contracts are controlled by a single Externally Owned Account (EOA: `0xb0935C6...`). | **Decentralized Multi-Sig / Timelock DAO:** Ownership transferred to a multi-party Safe or on-chain `Governance.sol` timelock contract. |

---

## 2. Proposed Architectural Phases

```
   PHASE 1: Dedicated Observer VM       PHASE 2: Public Bootnodes & P2P     PHASE 3: External Validator Kit
  ┌───────────────────────────────┐    ┌───────────────────────────────┐   ┌───────────────────────────────┐
  │ Edge Host (Caddy/UI/Faucet)   │    │ Static Bootnode Multiaddress  │   │ 1-Command Operator Docker     │
  │              │                │    │ /ip4/35.x/tcp/9000/p2p/<id>   │   │ Sepolia Staking + Identity    │
  │              ▼                │    │ Added to chain-spec.json      │   │ approveByConsensus Flow       │
  │ Dedicated Observer VM         │    │ Open firewall for 9000 TCP    │   │ Independent Operators Onboard │
  └───────────────────────────────┘    └───────────────────────────────┘   └───────────────────────────────┘
                                                       │
                                                       ▼
                                      PHASE 4: Witness Network & Governance
                                      ┌─────────────────────────────────────┐
                                      │ Independent Witness Daemons         │
                                      │ Safe Multi-Sig / Timelock DAO       │
                                      └─────────────────────────────────────┘
```

---

## Phase 1: Independent Observer Node Architecture

### Objective
Decouple the Observer Node from the Edge VM (`creg-testnet-vm`) and establish it on an independent, dedicated host (or multi-system pool).

### Changes Required:
1. **Provision Dedicated Observer VM (`creg-observer-vm`):**
   - Use `e2-medium` or `e2-standard-2` in a separate zone (e.g., `us-central1-a` or `us-central1-c`), separating fault domains from `creg-testnet-vm` (`us-central1-b`).
   - Run a standalone `creg-node` container in `observer` mode with persistent SSD volume for RocksDB state.
2. **Update Edge Gateway Caddy Routing:**
   - In `testnet/caddy/Caddyfile.unified`, update reverse-proxy targets for `/rpc`, `/jsonrpc`, `/v1/*`, and `/health` to point to the dedicated observer internal IP (`http://<OBSERVER_INTERNAL_IP>:8080`) instead of the local Docker container.
3. **High Availability Local Stack:**
   - Ensure the local development environment retains the HA dual-observer cluster (`docker-compose.observer-ha.yml`) with active Nginx load balancing on port 8090.

---

## Phase 2: Public Bootnodes & Network Discovery

### Objective
Enable independent nodes around the world to discover and peer with each other over libp2p.

### Changes Required:
1. **Expose P2P Listen Port:**
   - Configure the bootstrap/seed node to listen on public port `9000` (or `4001`) with TCP/UDP ingress.
   - Open GCP firewall rule `creg-allow-p2p` for port `9000` from `0.0.0.0/0`.
2. **Derive Peer ID & Multiaddress:**
   - Extract the libp2p Peer ID from the bootstrap node's identity key.
   - Format: `/dns4/bootnode.testnet.cregnet.dev/tcp/9000/p2p/<PEER_ID>` (or `/ip4/35.225.225.20/tcp/9000/p2p/<PEER_ID>`).
3. **Update Chain Spec:**
   - Populate `bootnodes` in `chain-spec.json`.
   - Re-sign the chain specification using `SPEC_SIGNING_KEY` from Google Secret Manager.
   - Deploy the new signed spec to `spec.testnet.cregnet.dev`.

---

## Phase 3: External Validator Operator Kit

### Objective
Empower external individuals and organizations to run independent validator nodes with zero maintainer intervention.

### Changes Required:
1. **Turnkey Validator Package (`deploy-external-validator.sh` / `.ps1`):**
   - Package `docker-compose.validator.yml` with a simple CLI installer.
   - Generates independent Ed25519 validator keys outside of maintainer access.
   - Guides the operator to stake 100 tCREG using the public faucet and `cast send` on Sepolia.
2. **Automated Admission Pipeline (`approveByConsensus`):**
   - Document and script the submission of the EIP-712 signed registration to active nodes.
   - Ensure active validators automatically verify stake on `Staking.sol` and sign the admission attestation bundle.
3. **Validator Explorer Dashboard:**
   - Expose active independent operators on `https://explorer.testnet.cregnet.dev/validators` showing distinct operator IDs, stake, and uptime.

---

## Phase 4: Independent Witness Network Activation

### Objective
Eliminate split-view risks by having external parties independently audit and cross-sign tree heads.

### Changes Required:
1. **Witness Daemon Deployment:**
   - Run `creg witness run --log-url https://api.testnet.cregnet.dev --interval 60s` on an external host.
2. **Roster Registration:**
   - Add the witness identity to `testnet/witnesses/roster.json`.
3. **Client Verification:**
   - Configure the developer CLI / resolver to check multiple witness URLs (`CREG_WITNESS_HEAD_URLS`) before accepting package manifests.

---

## Phase 5: Governance Decentralization

### Objective
Remove single-key authority over on-chain contracts.

### Changes Required:
1. **Safe Multi-Sig Deployment:**
   - Deploy a Gnosis Safe on Ethereum Sepolia with a minimum 2-of-3 or 3-of-5 threshold.
2. **Transfer Contract Ownership:**
   - Transfer `owner` on `Staking.sol`, `CREGToken.sol`, `Governance.sol`, and `Registry.sol` from `0xb0935C6...` to the Safe address.
3. **Timelock Binding:**
   - Route all protocol parameter changes through `Governance.sol` with a mandatory 48-hour timelock.
