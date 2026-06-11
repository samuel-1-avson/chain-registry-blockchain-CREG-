# Validator Onboarding Checklist

> Tracks: VAL-001 (admission policy), VAL-002 (operator checklist), MAL-001 (sandbox requirement)  
> Network: `creg-testnet-1` (Sepolia-anchored public alpha)  
> Audience: external validator operators joining the CREG testnet

This is the single runbook a new validator operator follows to register, stake, configure, and pass health checks. Completing every item in this document is a requirement for participating in public verification quorum.

## 1. Admission Policy (VAL-001)

A validator is admitted to the public testnet validator set when all of the following hold:

| Requirement | Threshold | How it is checked |
| --- | --- | --- |
| Stake | ≥ 100 tCREG staked as validator on the Sepolia Staking contract | `validator_set_sync` reads L1 state; explorer validator page |
| Identity | EVM address + node ID + Ed25519 pubkey registered and signed | `POST /v1/validators/register` (dual signature) |
| Consensus admission | Approved via `approveByConsensus` by an active validator | L1 event, then validator-set sync |
| Sandbox | Real behavioural sandbox (nsjail/gVisor/Docker). `CREG_DEV_SANDBOX=true` is **prohibited** | `/v1/runtime/config` → `sandbox_engine`, `sandbox_dev_bypass`; vote metadata |
| Scanner profile | Scanner ruleset digest matches the fleet profile (`testnet/scanner-fleet.env`) | `creg doctor` profile digest; consensus rejects mismatched evidence |
| Uptime | Node reachable and participating in rounds (target ≥ 95% during alpha) | Health endpoint polling, vote participation history |
| Hardware (minimum) | 4 vCPU / 8 GB RAM / 50 GB SSD, Linux with user namespaces enabled (nsjail) | Operator attestation; sandbox verification |
| Key hygiene | Validator Ed25519 key and EOA key stored per [SECURITY_OPS_RUNBOOK.md](./SECURITY_OPS_RUNBOOK.md); never committed | Operator attestation |

Votes from nodes that do not meet the sandbox and scanner-profile requirements are degraded/advisory and **do not count toward public verification quorum** (VAL-003).

## 2. Prerequisites

- [ ] Linux host (bare metal, VM, or Docker with Linux containers). Windows hosts are dev-only.
- [ ] Docker installed, or ability to run a release `creg-node` binary.
- [ ] Foundry `cast` installed (staking transactions).
- [ ] Sepolia RPC URL (Infura/Alchemy/QuickNode preferred — public RPCs break `eth_getLogs` validator-set sync).
- [ ] Sepolia ETH for gas and tCREG for stake (faucet: `creg testnet drip --address 0x...` when the operator faucet is up).
- [ ] Read [TESTNET_PHASE_SCOPE.md](./TESTNET_PHASE_SCOPE.md) and [PUBLIC_TESTNET_QUICKSTART.md](./PUBLIC_TESTNET_QUICKSTART.md) — understand what "verified" means today.

Contract addresses (Sepolia defaults):

| Contract | Address |
| --- | --- |
| Staking | `0xf28C63C4Aafd27025E535Ab9ab7B4daC18C96Bc2` |
| CREG Token | `0x97c21d46B3eac604e92E907D54aA92eEc0Af550b` |
| Registry | `0x3aCfF05d00AC199412a94326eD8aA874aaA3596c` |

## 3. Keys

Two distinct key types — do not mix them (see [WALLET_KEY_DERIVATION.md](./WALLET_KEY_DERIVATION.md)):

- [ ] **Ed25519 consensus key** — generate with `creg keygen validator --out ~/.creg/validator.key`. Signs votes and identity registration.
- [ ] **secp256k1 EOA** — a normal Ethereum wallet. Holds tCREG, signs staking transactions.
- [ ] Both keys stored outside the repo, never in shell history or compose files committed to git.
- [ ] Backup of both keys in a secure location (loss of the Ed25519 key requires re-registration; loss of the EOA strands the stake).

## 4. Stake And Register

- [ ] Stake as validator:

```bash
export SEPOLIA_RPC_URL=https://sepolia.infura.io/v3/YOUR_KEY
creg stake --amount 100 --role validator \
  --key ~/.creg/validator-eoa.key \
  --rpc-url "$SEPOLIA_RPC_URL"
```

- [ ] Register identity (binds EVM address, node ID, Ed25519 pubkey with both signatures):

```bash
# POST /v1/validators/register on the public API
# See /v1/runtime/config → validator_registration_note for the exact payload
export CREG_NODE_URL=https://api.testnet.cregnet.dev
```

- [ ] Request consensus admission (an active validator runs `approveByConsensus`); coordinate via the support channel in `chain-spec.sepolia.json` → `support.issues`.
- [ ] Confirm membership: your address appears in the explorer validator list and `validator_set_sync_state` is healthy.

## 5. Configure The Node

- [ ] Required environment:

| Variable | Value |
| --- | --- |
| `CREG_IS_VALIDATOR` | `true` |
| `CREG_VALIDATOR_KEY` | Ed25519 private key (hex) |
| `CREG_NODE_ID` | Your registered node ID |
| `CREG_CHAIN_ID` | `creg-testnet-1` |
| `CREG_TESTNET` | `true` |
| `CREG_DEV_SANDBOX` | **unset or `false`** — never `true` on a public validator |
| `CREG_CHAIN_SPEC_URL` | Signed spec, e.g. `https://spec.testnet.cregnet.dev/chain-spec.json` |
| `CREG_SPEC_SIGNING_PUBKEY` | From the operator docs (verifies spec signature) |
| `CREG_ETH_RPC` | Archive-capable Sepolia RPC |
| `CREG_IPFS_URL` | Local Kubo or operator IPFS API |
| `CREG_EXPECTED_L1_CHAIN_ID` | `11155111` |

- [ ] Scanner rules mounted and matching the fleet profile (`rules/` directory, `testnet/scanner-fleet.env`).
- [ ] Recommended: pin the IPFS CIDs of packages you validate (IPFS-005) — keeps verified content retrievable.

## 6. Sandbox Verification (MAL-001 — mandatory)

- [ ] Run with a real sandbox engine. Preferred: nsjail secure image (`Dockerfile.secure`, see `testnet/build-3node-secure-image.sh`); fleet operators get this by default from `start-validator-fleet-gcp.sh`.
- [ ] Verify the engine in-process:

```bash
curl -s "$CREG_NODE_URL/v1/runtime/config" | jq '{sandbox_engine, sandbox_dev_bypass}'
# Required: sandbox_engine one of nsjail|gvisor|docker, sandbox_dev_bypass=false
```

- [ ] `creg doctor` passes — it fails on `CREG_DEV_SANDBOX=true`.
- [ ] No `dev-bypass` lines in node logs after startup.

A validator running the dev bypass produces SB012 (High) findings on every package and its votes are not consensus-grade.

## 7. Health Checks

- [ ] API reachable: `curl -fsS http://<node>:8080/healthz` (or your mapped port).
- [ ] P2P connected: peer count > 0 in `/v1/chain/stats`.
- [ ] `validator_set_sync_state` not `degraded` (if degraded, switch to an archive RPC).
- [ ] Block height advancing and finalization lag bounded.
- [ ] Vote participation: your node appears in vote records for new packages.
- [ ] Clock synced (NTP) — consensus timestamps drift otherwise.

## 8. Operations

- [ ] Read [SECURITY_OPS_RUNBOOK.md](./SECURITY_OPS_RUNBOOK.md) — hot-key handling and rotation procedure.
- [ ] Read [INCIDENT_RESPONSE_RUNBOOK.md](./INCIDENT_RESPONSE_RUNBOOK.md) — know how revocation and key-compromise response work before you need them.
- [ ] Set up log retention (≥ 7 days) — vote evidence may be requested during disputes.
- [ ] Subscribe to operator announcements (GitHub Issues / support channel) for chain-spec updates and emergency notices.
- [ ] Plan upgrades: validators are expected to update within 72h of a security release during alpha.

## 9. Exit / Deregistration

- [ ] Announce planned exit in the operator channel (avoid surprise quorum loss).
- [ ] Stop the node, then unstake via the Staking contract after the unbonding window.
- [ ] Keys that held stake or signed votes follow the rotation/retirement procedure in the security runbook.

## Sign-Off

Operator onboarding is complete when:

1. All checklist items above are checked.
2. The node has participated in at least one verification round with a consensus-grade vote.
3. Sandbox evidence (`/v1/runtime/config` output or `verify-fleet-sandbox.ps1` JSON) is archived.

Maintainers record completion in [NEXT_WORK.md](./NEXT_WORK.md) (VAL-002 row) — completion by at least one non-core operator is an L2 Public Alpha gate.
