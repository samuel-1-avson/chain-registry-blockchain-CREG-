# Architecture Reference

## Crate dependency graph

```
                    ┌─────────────┐
                    │   common    │  ← shared types, no I/O
                    └──────┬──────┘
           ┌───────────────┼────────────────────┐
           ▼               ▼                    ▼
     ┌──────────┐   ┌────────────┐   ┌───────────────┐
     │ resolver │   │ validator  │   │   consensus   │
     └────┬─────┘   └─────┬──────┘   └──────┬────────┘
          │               │                 │
          └───────────────┼─────────────────┘
                          ▼
                    ┌──────────┐
                    │   node   │  ← REST API + P2P + Bridge
                    └──────────┘
                          ▲
                    ┌──────────┐
                    │   cli    │  ← creg binary + PATH shims
                    └──────────┘
```

## Data flow: package submission to verified install

```
Publisher
  │
  │  creg publish ./pkg-1.0.0.tgz
  │
  ▼
CLI (publish.rs)
  ├── sha256(tarball) → content_hash
  ├── IPFS pin → ipfs_cid
  ├── PGP sign (optional) → pgp_signature
  ├── Ed25519 sign(canonical || content_hash) → signature
  └── POST /v1/packages → Node REST API
                               │
                               ▼
                         Pending Pool
                         (in-memory, verified by validators)
                               │
                               │  validator_pipeline polls new submissions
                               ▼
                         Fetch tarball from IPFS
                               │
                               ▼
                   ┌───────────────────────┐
                   │    libp2p Swarm       │  ← Kademlia DHT +
                   │    Gossipsub PubSub   │    Peer Discovery
                   └───────────────────────┘
                               │
              ┌────────────────┼────────────────┐
              │                │                │
              ▼                ▼                ▼
        Stage 1            Stage 2          Stage 3
     Static Analysis    Sandbox Exec      Diff Analysis
     (AST + Entropy)    (nsjail/hook)    (vs prev version)
              │                │                │
              └────────────────┼────────────────┘
                               │
                               ▼
                     BFT Consensus Gate
                     (2/3+1 majority validator votes)
                               │
                ┌──────────────┴──────────────┐
                │ PASS                        │ FAIL
                ▼                             ▼
          Write Verified block          Write Revoke block
          to sled store                 + slash publisher
          Broadcast to P2P              Broadcast to P2P
                │
                ▼
          Package is now VERIFIED
          Synced to Ethereum Bridge
          (available to Explorer & CLI)

Developer
  │
  │  npm install express
  │
  ▼
npm PATH shim
  ├── resolver::resolve_id(PackageId)
  │     ├── cache::get() → hit? return immediately
  │     └── chain_client::fetch_verdict() → Node REST API GET /v1/packages/:canonical
  │           └── chain_store::get_package() → ChainRecord with status
  │
  ├── VerdictStatus::Verified  → proceed silently ✓
  ├── VerdictStatus::Unverified → warn + prompt (or --unverified flag)
  ├── VerdictStatus::Revoked   → hard block ✗
  └── VerdictStatus::Unknown   → warn + prompt (or --unverified flag)
        │
        ▼
  Call real npm (second in PATH)
```

## Block structure

```rust
Block {
    header: BlockHeader {
        height:             u64,
        prev_hash:          String,   // SHA-256 of previous block
        merkle_root:        String,   // Merkle root of transactions
        proposer_id:        String,   // Node ID of the block proposer
        timestamp:          DateTime<Utc>,
        validator_set_hash: String,   // Hash of active validators
    },
    transactions: Vec<Transaction>,
}

Transaction (enum):
  Publish(ChainRecord)               // new verified package
  Revoke { canonical, reason ... }   // package revocation
  ValidatorJoin { id, pubkey, stake } // new validator
```

## P2P & Consensus (libp2p)

The chain registry uses `libp2p` (v0.53) for all inter-node communication.
- **Transport**: TCP + Noise encryption + Yamux multiplexing.
- **Topology**: Decentralized Kademlia DHT for peer routing.
- **Propagation**: Gossipsub for consensus votes and block distribution.
- **Consensus**: BFT implementation ensuring state finality with 2/3 majority.

## Ethereum Bridge (Alloy)

Consensus finality is anchored to Ethereum for long-term immutability:
1. Validator nodes track block heights and signatures.
2. Signatures are batched and submitted to the `Registry` contract on-chain.
3. The bridge uses the `alloy` engine (v0.1) for high-performance EVM interactions.
4. Clients verify off-chain data against the on-chain root for absolute trust.

## Security Validation Scoring (Validator Crate)

| Finding | Severity | Action |
28: 178: | eval() / execSync() | Critical | Hard reject |
179: | Obfuscated base64 | High | Reject unless appealed |
180: | Undeclared network call | High | Reject unless excused by manifest |
181: | Undeclared child process | Critical | Hard reject |
182: | High entropy strings | High | Reject unless appealed |
183: | Typosquatting signatures | Critical | Hard reject |
184: | Undeclared FS write | High | Reject unless excused by manifest |
185: | process.env access | Low | Info only |

## Modernized Explorer

The **Blockchain Explorer** provides real-time visibility into:
- **Block Feed**: Live stream of verified packages and consensus updates.
- **P2P Visualizer**: Real-time graph of the libp2p swarm connectivity.
- **Bridge Monitor**: Tracking finality status on the Ethereum network.
- **Security Deep-Dive**: Forensic reports (Static/Sandbox/Diff) for every package.

## Adding a new ecosystem

1. Add a new shim binary in `crates/cli/src/shims/<name>.rs`
2. Register it in `SHIM_TARGETS` in `intercept.rs`
3. Add a `[[bin]]` entry to `crates/cli/Cargo.toml`
4. Teach `detect_ecosystem()` in `install.rs` to recognise the project file
5. Teach `detect_package_id()` in `publish.rs` to read the package manifest
6. Add the ecosystem string to any chain node routing logic
