# CREG Testnet Reboot — Progress Report

Date: 2026-09-22. Covers the faucet fix, DNS fix, stall diagnosis, and the
in-progress decentralized reboot (re-genesis + signing-key rotation).
No secrets in this file: pubkeys, peer IDs, addresses, and IPs only.

## 1. Current State Of The Blockchain

### CREG native chain: FROZEN at genesis (the problem being fixed)
- Height 0, 1 block (genesis only), 0 packages, 0 transactions.
- Cause (proven): the 4 genesis validators in the chain spec are phantoms —
  private keys lost, nodes never existed. Observer logs show the chain-spec
  bootnodes failing forever (`:9001` timeout; `:9000` peer-ID mismatch because
  the node dials itself). Sync status: `stalled, stall_polls>20000`.
- Quorum math makes revival impossible: served set is 4 phantoms (bootstrap
  members are re-merged every startup by design), PBFT needs 3 of 4, live = 0.
  Registering a new validator would make it 4-of-5 — still dead. Re-genesis
  is the only path.

### Sepolia L1: HEALTHY
- All contracts live: token `0x167A…`, staking `0x3060…`, registry `0xD0F4…`.
- Operator wallet `0x5f42…a311` is an **Active** L1 validator (100 tCREG,
  since 2026-09-19) — the only active validator on the contract.
- Second operator wallet `0xf4c0…D58e` designated for validator-2, not yet staked.

### Cloud services: ALL UP
- Explorer, API gateway, spec server, IPFS, faucet: healthy behind Caddy TLS.
- Faucet is tokens-only (1000 tCREG drips, 0 ETH) — live, user-verified.
- `rpc.testnet.cregnet.dev` DNS gap closed: A record added, Let's Encrypt
  cert issued, gateway JSON serving.
- Observer VMs (`creg-testnet-vm` local node + private `creg-observer-vm`
  `10.128.0.41`) agree: same frozen genesis, L1-following cursors current.

### Explorer behavior (explained, not a UI bug)
- Chain pages (blocks/txs/addresses) are empty because the chain is empty.
- Wallet view shows Sepolia balances only; Sepolia transactions (e.g. the
  user's `approve(staking, 100 tCREG)` `0xea54…acbb1`) never appear on CREG
  pages by design — they live on Sepolia Etherscan.
- User's validator identity 404s on the node API because the frozen chain
  never admitted it (no identity registration + no chain progress).

## 2. What Has Been Done

### Fixes shipped (pushed to `main`)
| Commit | Change |
|---|---|
| `c074464` | Faucet tokens-only default (`FAUCET_NATIVE_DRIP_AMOUNT=0`) |
| `dff61d8` | External-validator scripts: overridable `BOOTNODE`, PBFT small-cluster flag default, correct Dockerfile fallback |
| `d839e50` | Track `models/` + `validators/` so fresh clones build |
| `4574a6d` | Exec bit on `start-external-validator.sh` |
| `94b1aff` | Validator container runs as host user (RocksDB perms) |
| `be2e318` | Pre-create `chain-data/` dir (root-owned mount fix) |
| `81053d7` | `derive-peer-id.py` ceremony helper (verified vs live peer ID) |
| `5dfdb5c` | `register-validator.js` identity tool (verified vs Rust keygen) |
| `a7f383e` | `sign-chain-spec.js` spec signer (verified both directions vs repo validator) |
| `418521c` | Dockerfile pins new spec-signing pubkey by default |

### Reboot ceremony artifacts (collected, in `chain-spec.genesis2.draft.json`)
- validator-1 (cloud `creg-validator-1`, `34.74.35.254`, us-east1-b):
  node-id `validator-1`, eth `0x5f42…a311` (L1-active),
  Ed25519 `b3068f39…550c9`, peer `12D3KooWADE…44Vy` (bootnode).
- validator-2 (local workstation, home NAT):
  node-id `validator-2`, eth `0xf4c0…D58e` (to be staked),
  Ed25519 `ab6d12be…0643ab`, peer `12D3KooWDJc9…zY5UEF` (member-only).
- Bootnode 2: main-VM observer `12D3KooWSUs…myF8UXx` @ `35.225.225.20:9000`
  (identity preserved through wipe).
- New spec-signing pubkey: `51ff9f3e…73bd` (secret with operator only).

### Infrastructure + findings
- `creg-validator-1` (e2-medium, us-east1-b) created + running; firewall
  rule `creg-validator-p2p` (tcp/9000) live.
- Quotas found: us-central1 external IPs 4/4, global CPUs 12/12 → reboot
  goes with 2 validators now (2-of-2 quorum), 3rd joins when quota lands.
- Fresh `docker build` is broken (bindgen/libclang in `librocksdb-sys`):
  the main-VM fresh-clone build of `418521c` failed identically, so the new
  pin could not be baked in by rebuild. Workaround: binary pin-patch of the
  proven Sept-19 image — the old pin occurred 3x in `creg-node`, all
  replaced with the new pubkey (same length, `perl -pi`), repacked as
  `creg-node:genesis2`, and boot-tested: it rejects the current spec with
  `signing key 3fe8… does not match the binary-pinned …51ff…`, proving the
  new pin is active. Dockerfile root fix queued post-reboot.
- Draft spec `signing.signing_key_pubkey_hex` updated to the new key
  `51ff9f3e…73bd`, then finalized as `chain-spec.sepolia.json`:
  `genesis_time` 2026-09-23T06:54:48Z, Sepolia anchor 11784761,
  genesis hash `0x66aa3c35…95e0f` (repo CLI), detached sig `4306fc43…b4af0e`
  (user-signed on main VM, verified `✓ Ed25519 signature valid` +
  tamper control fails closed). Code-verified: spec_version must stay 1,
  2 validators → PBFT quorum exactly 2, P2P identity in
  `/data/p2p/identity.key` (preserve on wipe), seeds comma-separated.
- Validator launch scripts (`.sh`/`.ps1`) now prefer `creg-node:genesis2`
  and default the seed to the main observer (was a stale dead peer);
  unified compose node + faucet moved to `creg-node:genesis2`.
- Mystery processes solved: `35.224.170.75` = Cloud NAT egress (no box);
  `154.162.92.123` dialer = old local `ext-val-primary` container (stopped,
  Sept-19 key backed up); `fervent_dhawan` = stray debug container (removed);
  `cv-operator-*` = unrelated CipherVault project.
- Two validator secrets + one GitHub PAT were pasted into chat by accident
  during the ceremony; all were rotated/revoked. Current keys are clean.

## 3. In Progress / Next Steps

1. **Main-VM checkout quarantine** (done): tree had diverged ~400
   files + unique untracked installers; moved aside, fresh clone of
   `418521c`, carried over `.env.sepolia` + installers.
2. **`creg-node:genesis2` image** (done): pin-patched + boot-verified (see above).
3. **Distribute image** to validator-1 + local (done, `db25478b3f19`);
   `10.128.0.41` gets it at restart.
4. **Finalize spec** (done): timestamps, height 11784761, hash `0x66aa…e0f`.
5. **Sign + publish** (signed ✓, committed; publish = main-VM `git pull`).
6. **Coordinated restart** of all 4 nodes (wipes preserve `p2p/identity.key`).
   Order: main observer → validator-1 → validator-2 → private observer.
7. **Verify**: 5s blocks, climbing tip, explorer fills, both validators register
   (`register-validator.js`), validator-2 L1 staking, 3rd validator on quota.

## 4. Open Risks / Follow-ups

- Rotation fleet rollout uses the pin-patched image (no fresh compile);
  the Dockerfile bindgen failure is queued as a post-reboot fix.
- 2-of-2 quorum until the 3rd validator joins: either genesis box down = chain
  pauses (recovers automatically, no data loss).
- Post-reboot: Dockerfile bindgen fix, validator-2 L1 stake, triage of the
  preserved VM tree into git, one-line installer packaging.
