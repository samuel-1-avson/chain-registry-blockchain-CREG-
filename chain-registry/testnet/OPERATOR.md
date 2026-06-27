# Chain Registry testnet — operator runbook

**Network:** Sepolia (`creg-testnet-1`, L1 chain id `11155111`)  
**Scale model:** 10,000+ independent validator operators — see [TESTNET_SCALE_ARCHITECTURE.md](../../docs/TESTNET_SCALE_ARCHITECTURE.md)

This runbook is for **maintainers** operating edge services, optional seed validators, and cohort onboarding. External validators follow [VALIDATOR_ONBOARDING_CHECKLIST.md](../../docs/VALIDATOR_ONBOARDING_CHECKLIST.md).

---

## Topology

| Tier | Where | Purpose |
|------|-------|---------|
| **Edge** | `creg-testnet-vm` (public) | TLS, spec server, IPFS, faucet, waitlist/hub proxies |
| **Observer pool** | GCP MIG (optional) | Public API reads (`api.testnet.cregnet.dev`) |
| **Seed validators** | `creg-validator-vm` (private VPC) | Optional bootstrap quorum — **not** the validator cap |
| **External validators** | Operator-owned hosts | Genesis Alpha cohort — one host per operator |
| **Sepolia RPC** | `creg-sepolia-geth-vm` (private) | `CREG_ETH_RPC` for fleet and observers |

Cohort waves: [GENESIS_ALPHA_COHORT_PLAYBOOK.md](../../docs/GENESIS_ALPHA_COHORT_PLAYBOOK.md).

**Retired:** Sepolia `docker-compose.3node.yml` lab — archived under [archive/3node-lab/](./archive/3node-lab/README.md). Do not use for new work.

---

## Maintainer seed fleet (optional)

Small maintainer fleet on GCP seeds P2P and early quorum while external operators join.

```powershell
.\testnet\init-sepolia-fleet-env.ps1
# Set CREG_EDGE_INTERNAL_IP, CREG_ETH_RPC (internal Geth) in testnet/sepolia-fleet.env
```

On `creg-validator-vm` (Linux):

```bash
./testnet/start-validator-fleet-gcp.sh
```

Compose: `docker-compose.validator-fleet.yml` + `docker-compose.fleet-sandbox.yml` (nsjail default).

Deploy from workstation:

```powershell
.\testnet\gcp\deploy-validator-fleet.ps1
.\testnet\gcp\verify-fleet-sandbox.ps1
```

Detail: [GCP-VALIDATOR-FLEET.md](../../docs/GCP-VALIDATOR-FLEET.md).

---

## External validator onboarding

Each operator runs **one validator per machine** using `docker-compose.validator.yml` or the release `creg-node` binary.

| Step | Reference |
|------|-----------|
| Cohort invite | [GENESIS_ALPHA_COHORT_PLAYBOOK.md](../../docs/GENESIS_ALPHA_COHORT_PLAYBOOK.md) |
| Full checklist | [VALIDATOR_ONBOARDING_CHECKLIST.md](../../docs/VALIDATOR_ONBOARDING_CHECKLIST.md) |
| Publisher/dev flows | [PUBLIC_TESTNET_QUICKSTART.md](../../docs/PUBLIC_TESTNET_QUICKSTART.md) |
| Alpha limits | [TESTNET_PHASE_SCOPE.md](../../docs/TESTNET_PHASE_SCOPE.md) |

**NET-301 acceptance (scale):** L1 shows ≥2 active staked validators from **independent operators**, `CREG_PBFT_ALLOW_SMALL_CLUSTER_QUORUM` unset on public hosts, publish reaches `verified` with PBFT quorum — not single-validator override.

---

## Sandbox (SANDBOX-301 / MAL-001)

| Profile | Image | `CREG_DEV_SANDBOX` | Host |
|---------|-------|-------------------|------|
| Public validator | `Dockerfile.secure` (nsjail) | `false` | Linux |
| Maintainer seed fleet | `chain-registry-node-secure:fleet` | `false` (forced by fleet-sandbox overlay) | `creg-validator-vm` |
| Local Sepolia bootstrap | `docker-compose.testnet.yml` | Sepolia L1 | Local |

```powershell
.\testnet\build-3node-secure-image.ps1   # builds nsjail secure image (name legacy; used by fleet)
.\testnet\sandbox-301-verify.ps1
```

Public validator profiles must never run `CREG_DEV_SANDBOX=true`.

---

## IPFS pinning (IPFS-001 / IPFS-002)

```powershell
.\testnet\gcp\run-ipfs-pin-check.ps1
.\testnet\gcp\run-ipfs-pin-check.ps1 -CheckOnly
```

Reports: `testnet/ipfs-pin-logs/`. Non-zero exit → Scenario 4 in [INCIDENT_RESPONSE_RUNBOOK.md](../../docs/INCIDENT_RESPONSE_RUNBOOK.md).

---

## Distribution (DIST-301)

```bash
git tag v0.1.2-testnet && git push origin v0.1.2-testnet
```

```powershell
.\testnet\verify-dist-301.ps1 -Version v0.1.2-testnet
```

---

## Sepolia L1 JSON-RPC (public)

| URL | Purpose |
|-----|---------|
| `https://explorer.<base>/rpc` | Sepolia `eth_*` for wallets and `cast` |
| `https://api.<base>/rpc` | **CREG only** — not Sepolia |

Verify: `.\testnet\verify-sepolia-rpc-endpoints.ps1`

---

## L1 anchoring & reorg safety

| Knob | Default | Meaning |
|------|---------|---------|
| `CREG_VALIDATOR_SET_FINALITY_LAG` | `2` (testnet) | Blocks before applying L1 staking events |
| `CREG_BRIDGE_SELF_APPROVE` | `true` | Set `false` for multi-signer governance anchor path |

Anchors: `GET /v1/bridge/anchors`. Reorgs: `GET /v1/reorgs`.

---

## Routine operations

| Task | Command |
|------|---------|
| Resume public hosting | `.\testnet\gcp\resume-testnet-hosting.ps1` |
| Start seed fleet (VM) | `./testnet/start-validator-fleet-gcp.sh` |
| Hosting verify | `.\testnet\hosting-301-verify.ps1 -BaseDomain testnet.cregnet.dev` |
| Stop hosting (cost) | `.\testnet\gcp\stop-testnet-hosting.ps1` |
| Local CI | `cargo test --workspace`; `cd contracts && forge test` |

---

## Public hosting (HOSTING-301)

Runbook: [gcp/README.md](./gcp/README.md)

```powershell
.\testnet\prepare-public-hosting.ps1 -BaseDomain testnet.YOUR_DOMAIN -AcmeEmail you@example.com
.\testnet\hosting-301-verify.ps1 -BaseDomain testnet.YOUR_DOMAIN
```

---

## References

- [TESTNET_SCALE_ARCHITECTURE.md](../../docs/TESTNET_SCALE_ARCHITECTURE.md)
- [TESTNET_SEPOLIA_RUNBOOK.md](../../docs/TESTNET_SEPOLIA_RUNBOOK.md)
- [WHAT_TO_DO_NEXT.md](../../docs/WHAT_TO_DO_NEXT.md)
- [GCP-VALIDATOR-FLEET.md](../../docs/GCP-VALIDATOR-FLEET.md)