# Chain Registry testnet — operator runbook

**Network:** Sepolia (`creg-testnet-1`, L1 chain id `11155111`)  
**Fleet:** 2 validators + 1 observer via `docker-compose.3node.yml`

---

## Topology (NET-301)

| Node | Container | Role | `CREG_NODE_ID` | API (default host port) |
|------|-----------|------|----------------|-------------------------|
| 1 | `creg-3node-node1` | Validator (bootstrap) | `core-1` | `28180` |
| 2 | `creg-3node-node2` | Validator | `validator-2` | `28181` |
| 3 | `creg-3node-node3` | Observer | `observer-1` | `28182` |

Shared: IPFS (`15001`), chain-spec nginx (`18888`), P2P `29100–29102`.

**Multi-host deployment:** Run one validator per machine using `docker-compose.validator.yml` with the same `chain-spec.sepolia.json`, distinct `CREG_VALIDATOR_KEY`, and P2P bootnodes pointing at peers. Each host needs Sepolia RPC, IPFS (shared or per-node with pin sync), and on-chain stake for its validator EOA.

**NET-301 acceptance:** ≥2 validators active on L1, `CREG_PBFT_ALLOW_SMALL_CLUSTER_QUORUM` unset, publish reaches `verified` with PBFT quorum — not single-validator override.

```powershell
# Same-machine lab (compose)
.\testnet\init-sepolia-3node-env.ps1
.\testnet\start-3node-test.ps1
$env:VALIDATOR_2_ETH_PRIVATE_KEY = "0x..."   # never commit
.\testnet\register-validator-2-sepolia.ps1
.\testnet\net-301-quorum-verify.ps1
```

---

## Sandbox (SANDBOX-301)

| Profile | Image | `CREG_DEV_SANDBOX` | Host |
|---------|-------|-------------------|------|
| Windows dev soak | `Dockerfile.windows` | `true` optional | Windows |
| Credible testnet | `Dockerfile.secure` (nsjail) | `false` | Linux container backend (Docker Desktop WSL2 on Windows qualifies) |

Linux secure fleet:

```powershell
.\testnet\build-3node-secure-image.ps1          # or -SkipAppBuild if app image is current
.\testnet\start-3node-sandbox.ps1
.\testnet\sandbox-301-verify.ps1                # or soak-3node-sandbox.ps1
```

Build images manually: `.\testnet\build-3node-secure-image.ps1` (or `build-3node-secure-image.sh` on Linux). After validator code changes, rebuild with `-RebuildApp`.

Reference privileged profile: `docker-compose.testnet.yml` (`node-1` uses `chain-registry-node-secure:latest`).

---

## Distribution (DIST-301)

Maintainers tag and push; CI publishes binaries:

```bash
git tag v0.1.0-testnet
git push origin v0.1.0-testnet
```

Verify release + install URL:

```powershell
.\testnet\verify-dist-301.ps1 -Version v0.1.0-testnet
.\testnet\verify-dist-301.ps1 -Version v0.1.0-testnet -RunInstallSh   # Linux/Git Bash
```

Install (uses `CREG_GITHUB_REPO` or git `origin`):

```bash
export CREG_GITHUB_REPO=samuel-1-avson/chain-registry-blockchain-CREG-
./scripts/install-creg.sh --version v0.1.0-testnet
```

---

## Routine operations

| Task | Command |
|------|---------|
| Start fleet | `.\testnet\start-3node-test.ps1` |
| Soak (parity + publish) | `.\testnet\soak-3node-consensus.ps1` |
| Stop | `docker compose -f testnet/docker-compose.3node.yml --env-file testnet/sepolia-3node.env down` |
| Health | `Invoke-RestMethod http://localhost:28180/v1/health` |
| Logs | `docker compose -f testnet/docker-compose.3node.yml logs -f creg-node-1` |

---

## Security audit (SEC-401)

Scope: [docs/SEC-401-AUDIT-SCOPE.md](../../docs/SEC-401-AUDIT-SCOPE.md)  
Outreach template: [docs/SEC-401-VENDOR-OUTREACH.md](../../docs/SEC-401-VENDOR-OUTREACH.md)

Generate send-ready email: `.\testnet\prepare-sec-401-outreach.ps1` → `docs/SEC-401-outreach-ready.md` (pins `v0.1.0-testnet` SHA).

Record vendor and **start date** in [docs/NEXT_WORK.md](../../docs/NEXT_WORK.md) when booked.

## Public hosting (HOSTING-301)

Runbook: [gcp-public-hosting.md](./gcp-public-hosting.md)

```powershell
.\testnet\prepare-public-hosting.ps1 -BaseDomain testnet.YOUR_DOMAIN -AcmeEmail you@example.com
# On GCP VM: ./testnet/start-3node-gcp.sh
.\testnet\hosting-301-verify.ps1 -BaseDomain testnet.YOUR_DOMAIN
```

---

## References

- [TESTNET_SEPOLIA_RUNBOOK.md](../../docs/TESTNET_SEPOLIA_RUNBOOK.md)
- [TESTNET_READINESS_REPORT.md](../TESTNET_READINESS_REPORT.md)
- [NEXT_WORK.md](../../docs/NEXT_WORK.md)
