# Validator Onboarding Checklist

> Tracks: VAL-001 (admission policy), VAL-002 (operator checklist), MAL-001 (sandbox requirement)  
> Network: `creg-testnet-1` (Sepolia-anchored public alpha)  
> Audience: external validator operators joining the CREG testnet
>
> **Current reality (2026-08-23):** the signed Sepolia spec records four
> coordinated validators (`validator-1`…`validator-4`) and no bootnodes.
> The public GCP fleet is paused. External admission is not represented as
> open or complete. This is a recruitment/readiness checklist until operators
> publish a signed onboarding window and independently verify the live fleet.

This is the single runbook a new validator operator follows to register, stake, configure, and pass health checks. Completing every item in this document is a requirement for participating in public verification quorum.

Running several nodes for the same person or organization does not create
Byzantine fault tolerance. An operator counts as independent only when its
governance, validator keys, host/cloud account, deployment credentials,
monitoring, and incident decision path are not controlled by an existing
operator. Control-domain disclosures are retained with the onboarding record.

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
| Independent control | Governance, keys, infrastructure, monitoring, and incident authority disclosed and independently controlled | Two-person review of retained control-domain statement |
| Certified history | Exact independently verified weak-subjectivity checkpoint and descendant history accepted | Checkpoint evidence plus rebuilt-node rehearsal |

Votes from nodes that do not meet the sandbox and scanner-profile requirements are degraded/advisory and **do not count toward public verification quorum** (VAL-003).

## 2. Prerequisites

- [ ] Linux host (bare metal, VM, or Docker with Linux containers). Windows hosts are dev-only.
- [ ] Docker installed, or ability to run a release `creg-node` binary.
- [ ] Foundry `cast` installed (staking transactions).
- [ ] Sepolia RPC URL (Infura/Alchemy/QuickNode preferred — public RPCs break `eth_getLogs` validator-set sync).
- [ ] Sepolia ETH for gas and tCREG for stake (faucet: `creg testnet drip --address 0x...` when the operator faucet is up).
- [ ] Read [TESTNET_PHASE_SCOPE.md](./TESTNET_PHASE_SCOPE.md) and [PUBLIC_TESTNET_QUICKSTART.md](./PUBLIC_TESTNET_QUICKSTART.md) — understand what "verified" means today.

Contract addresses recorded in the current repository Sepolia spec are shown
below. Verify them against the exact signature-verified chain spec supplied for
the onboarding window; this table is not an authority to transact:

| Contract | Address |
| --- | --- |
| Staking | `0xE906ae140987CC2cA711dDa4ee43574d3aDc31b7` |
| CREG Token | `0xB326dB1C6e457f4AdfBb11f17FDc145ED585b288` |
| Registry | `0xF232cC6F5C6EfC74C7517B209D15baBd57b817eE` |

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
# POST /v1/validators/register on a reachable node (the public API is paused).
# See /v1/runtime/config → validator_registration_note for the exact payload.
export CREG_NODE_URL=http://127.0.0.1:8080
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
| `CREG_SPEC_SIGNING_PUBKEY` | **Build-time pin only** (`build.rs` / `option_env`). Runtime env is ignored. Confirm the binary was built with `3fe87833177b04b0a18600eace041692a35fb2b85f6e14e6734d894a0fd61c01` |
| `CREG_P2P_SEEDS` | At least one dialable multiaddr (`CREG_VALIDATOR_P2P_SEEDS` is the fallback name) |
| `CREG_STAKING_ADDR` | Signed-spec staking contract (validators refuse to boot without a parseable address) |
| `CREG_YARA_RULES_DIR` | Directory of `.yar`/`.yara` rules (votes are `degraded-yara-missing` if empty) |
| `CREG_ETH_RPC` | Archive-capable Sepolia RPC (`CREG_ETH_RPC_URL` is the fallback name) |
| `CREG_IPFS_URL` | Local Kubo or operator IPFS API |
| `CREG_EXPECTED_L1_CHAIN_ID` | `11155111` |
| `CREG_WEAK_SUBJECTIVITY_CHECKPOINT` | Exact single-line checkpoint certificate independently approved for the onboarding window |
| `CREG_REQUIRE_CERTIFIED_COMMITTEES` | `true` only after the signed checkpoint rollout and fleet rehearsal; never enable by copying an anchor from one peer |

- [ ] Scanner rules mounted and matching the fleet profile (`rules/` directory, `testnet/scanner-fleet.env`).
- [ ] Recommended: pin the IPFS CIDs of packages you validate (IPFS-005) — keeps verified content retrievable.
- [ ] Compare `GET /v1/validator/set-transitions` with at least two independent
      peers and retain evidence that the configured checkpoint and all
      descendants agree.
- [ ] Rebuild an observer from the checkpoint before activating validator
      duties; missing, conflicting, fork-anchored, replayed, and long-range
      histories must fail closed.

CR-07 npm/PyPI authority attestation is a separate privileged role. A
validator MUST NOT configure either authority verifier unless the exact
self-contained verifier, policy, and trust-root hashes appear in the signed
chain spec and the CR-07 activation manifest is valid. A normal validator can
join while CR-07 remains disabled.

## 6. Bootnode role (optional and separately reviewed)

A validator is not automatically a bootnode. Operators volunteering a
bootnode additionally provide:

- a stable DNS name, static P2P endpoint, and exact peer ID for the signed
  chain spec;
- DDoS controls and resource limits without validator/operator API exposure;
- retained uptime, connection, restart, and abuse metrics;
- a key and infrastructure control domain distinct from the existing
  bootnodes; and
- a process-kill/restart and partition rehearsal showing that peers can still
  discover at least one independently controlled seed.

Placeholder peer IDs or DNS names are never added to an operational signed
spec. At least two independently controlled bootnodes are required before the
topology is described as resilient.

## 7. Sandbox Verification (MAL-001 — mandatory)

- [ ] Run with a real sandbox engine. Preferred: nsjail secure image (`Dockerfile.secure`, see `testnet/build-3node-secure-image.sh`). Each independent operator must verify the engine locally; GCP does not host validator containers.
- [ ] Verify the engine in-process:

```bash
curl -s "$CREG_NODE_URL/v1/runtime/config" | jq '{sandbox_engine, sandbox_dev_bypass}'
# Required: sandbox_engine one of nsjail|gvisor|docker, sandbox_dev_bypass=false
```

- [ ] `creg doctor` passes — it fails on `CREG_DEV_SANDBOX=true`.
- [ ] No `dev-bypass` lines in node logs after startup.

A validator running the dev bypass produces SB012 (High) findings on every package and its votes are not consensus-grade.

## 8. Health Checks

- [ ] API reachable: `curl -fsS http://<node>:8080/v1/health` (or your mapped port).
- [ ] P2P connected: peer count > 0 in `/v1/chain/stats`.
- [ ] `validator_set_sync_state` not `degraded` (if degraded, switch to an archive RPC).
- [ ] Block height advancing and finalization lag bounded.
- [ ] Vote participation: your node appears in vote records for new packages.
- [ ] Clock synced (NTP) — consensus timestamps drift otherwise.

## 9. Operations

- [ ] Read [SECURITY_OPS_RUNBOOK.md](./SECURITY_OPS_RUNBOOK.md) — hot-key handling and rotation procedure.
- [ ] Read [INCIDENT_RESPONSE_RUNBOOK.md](./INCIDENT_RESPONSE_RUNBOOK.md) — know how revocation and key-compromise response work before you need them.
- [ ] Set up log retention (≥ 7 days) — vote evidence may be requested during disputes.
- [ ] Subscribe to operator announcements (GitHub Issues / support channel) for chain-spec updates and emergency notices.
- [ ] Plan upgrades: validators are expected to update within 72h of a security release during alpha.
- [ ] Complete and retain the operator rehearsal matrix: transition replay,
      invalid quorum, partition/heal, fork/reorg anchor, rebuilt node,
      long-range alternative history, verifier crash/timeout (if applicable),
      and abrupt process kill during durable work.

## 10. Exit / Deregistration

- [ ] Announce planned exit in the operator channel (avoid surprise quorum loss).
- [ ] Stop the node, then unstake via the Staking contract after the unbonding window.
- [ ] Keys that held stake or signed votes follow the rotation/retirement procedure in the security runbook.

## Sign-Off

Operator onboarding is complete when:

1. All checklist items above are checked.
2. The node has participated in at least one verification round with a consensus-grade vote.
3. Sandbox evidence (`/v1/runtime/config` output or `verify-fleet-sandbox.ps1` JSON) is archived.
4. The independently reviewed control-domain statement, checkpoint evidence,
   signed-spec hash, binary hash, and rehearsal record are linked from the
   operator acceptance record.
5. At least one maintainer and one reviewer outside the candidate operator's
   control domain sign off. Repository documentation alone cannot supply this
   evidence.

Maintainers record completion in [NEXT_WORK.md](./NEXT_WORK.md) (VAL-002 row) — completion by at least one non-core operator is an L2 Public Alpha gate.
