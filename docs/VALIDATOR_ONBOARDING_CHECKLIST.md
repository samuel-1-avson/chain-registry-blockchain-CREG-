# Validator Onboarding Checklist

> Tracks: VAL-001 (admission policy), VAL-002 (operator checklist), MAL-001 (sandbox requirement)  
> Network: `creg-phase-c-testnet-2026-09-26-r7` (Sepolia-anchored public alpha)
> Audience: external validator operators joining the CREG testnet
>
> **Current status (2026-11-14):** the live network is the R7 testnet
> (`creg-phase-c-testnet-2026-09-26-r7`) with four validators, all run by one
> operator. The onboarding path below is documented but **has not yet been
> rehearsed with an independent operator**, and admission needs a two-thirds
> attestation quorum from the existing validators. Validator-set sync from
> Sepolia is currently degraded (pruned RPC history), so use an RPC with full
> log history. See [CREG-COMPLETE-DOCUMENTATION.md](./CREG-COMPLETE-DOCUMENTATION.md)
> §10 and §17.

This is the single runbook a new validator operator follows to register, stake, configure, and pass health checks. Completing every item in this document is a requirement for participating in public verification quorum.

## Guided R7 enrollment

- Windows Validator Console users can select **Open validator onboarding**. The console uses a `creg` binary with `validator onboard` support.
- In `creg console`, press `:` and select **Join R7 as a validator**. From the CLI, run `creg validator onboard`.

The flow opens a local browser page for an Ethereum wallet extension. The wallet approves the R7 token transfer and signs its half of the registration. The CLI keeps the enrollment key at `~/.creg-validator/onboarding.key`, signs the same challenge, and submits both proofs. Reopening the flow with that key restores the same validator identity. Keep the session open to see on-chain approval, local node sync, and any matching commit observed in recent finalized blocks.

Open the flow from the CREG checkout and select **Start local R7 sync**. The launcher uses the reviewed R7 image and scanner bundle to sync an observer with the registered node ID. The enrollment key stays on the host. If Docker cannot pull the image, import the maintainer-provided image. Existing operators can use `creg validator onboard --node-id <id> --key-file <path>` to watch their own validator. Keep an established validator's data and signing journal together.

The deployed R7 image starts voting nodes from the original signed roster. The external signing path in source also accepts a prior-set quorum certificate for the node's exact identity, anchored to its local canonical chain. A voting trial requires that runtime and a coordinated certified validator-set transition. Confirm observer readiness and the transition plan before changing governance or quorum.

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
| Consensus admission | Approved via 2-of-3 multisig governance or `approveByConsensus` | L1 event, then validator-set sync |
| Sandbox | Real behavioural sandbox (nsjail/gVisor/Docker). `CREG_DEV_SANDBOX=true` is **prohibited** | `/v1/runtime/config` → `sandbox_engine`, `sandbox_dev_bypass`; vote metadata |
| Scanner profile | Scanner ruleset digest matches the fleet profile (`testnet/scanner-fleet.env`) | `creg doctor` profile digest; consensus rejects mismatched evidence |
| Uptime | Node reachable and participating in rounds (target ≥ 95% during alpha) | Health endpoint polling, vote participation history |
| Hardware (minimum) | 4 vCPU / 8 GB RAM / 50 GB SSD, Linux or Windows Docker Desktop | Operator attestation; sandbox verification |
| Key hygiene | Validator Ed25519 key and EOA key stored per [SECURITY_OPS_RUNBOOK.md](./SECURITY_OPS_RUNBOOK.md); never committed | Operator attestation |
| Independent control | Governance, keys, infrastructure, monitoring, and incident authority disclosed and independently controlled | Two-person review of retained control-domain statement |
| Certified history | Exact independently verified weak-subjectivity checkpoint and descendant history accepted | Checkpoint evidence plus rebuilt-node rehearsal |

Votes from nodes that do not meet the sandbox and scanner-profile requirements are degraded/advisory and **do not count toward public verification quorum** (VAL-003).

## 2. Prerequisites

- [ ] Linux host (bare metal, VM, or Docker) or Windows host with Docker Desktop.
- [ ] Docker installed, or ability to run a release `creg-node` binary.
- [ ] Staking CLI tool (`creg stake` or `.\testnet\stake-validator.cmd`).
- [ ] A Sepolia RPC that can return staking logs from the R7 deployment onward. The reviewed observer profile uses `https://sepolia.gateway.tenderly.co`.
- [ ] Sepolia ETH for gas and tCREG for stake (faucet: `creg testnet drip --address 0x...` or web faucet).
- [ ] Read [PUBLIC_TESTNET_QUICKSTART.md](./PUBLIC_TESTNET_QUICKSTART.md).

Active contract addresses on Ethereum Sepolia for the R7 network (from the signed
`testnet/phase-c/r7/chain-spec.json`; always confirm against the spec you boot with):

| Contract | Active Address |
| --- | --- |
| Staking | `0x61c00EBe7C46f0bcE63942F7E15Bcc434DFC9B7C` |
| CREG Token (tCREG) | `0xbC2a72107C6765364D0A822a88A6e6C61f30EAa6` |
| Governance (2-of-3) | `0x0AB4Db22A9902B8Ef678aBd408889af22277d24E` |
| Registry | `0x23c7FB8e6fd932C1078Ff76244470557E5e6bd99` |

Addresses from the retired `creg-testnet-1` network (`0x3060…7331`, `0x167A…99E2`,
`0x0fC1…5D4E`, `0xD0F4…AF3B`) no longer apply.

## 3. Keys

Two distinct key types — do not mix them (see [WALLET_KEY_DERIVATION.md](./WALLET_KEY_DERIVATION.md)):

- [ ] **Ed25519 consensus key** — generate with `creg keygen validator --key-path ~/.creg/validator.key`. Signs votes and identity registration.
- [ ] **secp256k1 EOA** — a normal Ethereum wallet. Holds tCREG, signs staking transactions.
- [ ] Both keys stored outside the repo, never in shell history or compose files committed to git.
- [ ] Backup of both keys in a secure location (loss of the Ed25519 key requires re-registration; loss of the EOA strands the stake).

## 4. Stake And Register

- [ ] Stake as validator:

```bash
export SEPOLIA_RPC_URL=https://sepolia.infura.io/v3/YOUR_KEY
creg stake 100 --role validator \
  --key-file ~/.creg/validator-eoa.key \
  --rpc-url "$SEPOLIA_RPC_URL" \
  --staking-addr 0x61c00EBe7C46f0bcE63942F7E15Bcc434DFC9B7C \
  --token-addr 0xbC2a72107C6765364D0A822a88A6e6C61f30EAa6
```

- [ ] Register identity (binds EVM address, node ID, Ed25519 pubkey with both signatures):

```bash
# POST /v1/validators/register on a reachable node (the public API is paused).
# See /v1/runtime/config → validator_registration_note for the exact payload.
export CREG_NODE_URL=http://127.0.0.1:8080
```

- [ ] Coordinate with the maintainer to collect the signatures for `approveByConsensus` on R7.
- [ ] Confirm membership: your address appears in the explorer validator list and `validator_set_sync_state` is healthy.

### Prepare a consensus approval with existing validator wallets

An operator can collect Sepolia approvals from the wallets that already hold active R7 validator stakes. With four active validators, the contract requires three distinct wallet signatures. The wallet's signing key stays in its existing keystore or hardware wallet.

1. Confirm that the applicant's identity is registered and its observer has synced. Coordinate the certified validator-set transition and the signing-runtime rollout before collecting a quorum of approvals.
2. Export the current application, nonce, and membership epoch using the Windows rehearsal tool from the checkout root.

   ```powershell
   .\chain-registry\testnet\rehearse-validator-admission.ps1 -Applicant <applicant-wallet> -TypedDataPath .\r7-admission.json
   ```

   Pass `-SignerAddress` with the operators' public wallet addresses to check their active staking status.
3. Each operator verifies the applicant wallet and R7 staking address in the JSON and signs the typed data with their active Sepolia wallet. For example, an operator with a local encrypted Foundry account can run `cast wallet sign --data --from-file --account <account-name> .\r7-admission.json`. Collect the public signer address and the signature in a JSON array of objects with `signer` and `signature` fields. Keep the bundle with the designated submitter. Anyone holding a valid quorum bundle can submit it.
4. Run the rehearsal tool with the same `-Applicant` and `-SignaturesPath <bundle.json>`. It checks signer uniqueness, simulates the exact call against Sepolia, and prints the calldata when the contract accepts it. The designated submitter can use that calldata in an authorized Sepolia transaction to the reported staking contract. Regenerate the typed data if the validator-set epoch or application nonce changes.

### Enable automatic consensus approval

1. Deploy a reviewed R7 image that includes the new admission runtime to the validators and their P2P relays. The original R7 launch scripts pin an older image. Preserve each node's data and P2P identity, roll validators one at a time, and verify chain and staking sync after each restart.
2. Provision each participating validator's **own active Sepolia staking wallet** in its admission signer role. With Vault, set `CREG_VAULT_SECRET_PATH_VALIDATOR_ADMISSION` to that node's KV path. The role reads `validator_admission_key`. The testnet environment backend uses `CREG_VALIDATOR_ADMISSION_KEY`. Set only the signer first. The startup log **Validator admission signer verified in check-only mode** confirms the wallet matches that node's active staking identity and the signed Sepolia network.
3. Coordinate the candidate signing runtime and prepare a prior-set quorum-signed transition anchored to the current finalized CREG tip. The transition must add the exact registered wallet, node ID, and Ed25519 key while preserving the rest of the roster. Stage the same plan on at least three current R7 validators. The JSON fields are `applicant` and `submitter` (Sepolia wallet addresses), `application_nonce` and `validator_set_epoch` (JSON integers from the rehearsal), and `transition` (the full `AuthenticatedValidatorSetSnapshot`). Keep the transition staged until the coordinated activation.
4. Fund the designated submitter's active validator wallet with Sepolia ETH and set `CREG_VALIDATOR_ADMISSION_PLAN` to the staged file on the participating nodes. In Docker, use a path readable inside the container, such as `/data/admission-plan.json` on the existing data mount. On restart, the nodes validate the certificate and exchange EIP-712 attestations over CREG P2P. The designated wallet simulates the exact approval call before submitting it when quorum arrives. If the CREG tip moves before submission, stage a fresh plan anchored to the new tip and reload it on the participating nodes.
5. Verify the successful Sepolia receipt. Complete the certified validator-set transition and confirm the applicant's first vote before ending the trial.

## 5. Configure The Node

- [ ] Required environment:

| Variable | Value |
| --- | --- |
| `CREG_IS_VALIDATOR` | `true` |
| `CREG_VALIDATOR_KEY` | Ed25519 private key (hex) |
| `CREG_NODE_ID` | Your registered node ID |
| `CREG_CHAIN_ID` | `creg-phase-c-testnet-2026-09-26-r7` (adopted from the signed spec) |
| `CREG_TESTNET` | `true` |
| `CREG_DEV_SANDBOX` | **unset or `false`** — never `true` on a public validator |
| `CREG_CHAIN_SPEC_URL` | Signed spec, e.g. `https://spec.testnet.cregnet.dev/phase-c/r7/chain-spec.json` |
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

Maintainers record completion in [PROTOCOL_PROGRESS.md](./PROTOCOL_PROGRESS.md) ([NEXT_WORK.md](./archive/NEXT_WORK.md) archived 2026-09-19; VAL-002 row) — completion by at least one non-core operator is an L2 Public Alpha gate.
