# CREG testnet — live status

> **Last updated:** 2026-08-22
> **Single source of truth** for what is running today. Supersedes stale “phase open” wording in older docs until those pages are refreshed.

---

## Summary

| Surface | Status | Notes |
|---------|--------|-------|
| **Public GCP fleet** (`api.testnet.cregnet.dev`, validators, IPFS gateway) | **Paused (cost)** | Fleet can stay off until external validators are ready |
| **Waitlist** | **Live** | Firebase — [WAITLIST-ONLY-HOSTING.md](./WAITLIST-ONLY-HOSTING.md) |
| **Contribution Arena** | **Separate product (live)** | Educational/simulation only — not protocol work |
| **Release binaries** | **v0.4.3-testnet** | [GitHub release](https://github.com/samuel-1-avson/chain-registry-blockchain-CREG-/releases/tag/v0.4.3-testnet) |
| **Local / operator nodes** | **Available** | Run your own observer or validator against Sepolia chain spec |
| **Protocol code & docs** | **Active** | Public alpha; not mainnet |

## Readiness gate update (2026-08-01)

The repository now contains authenticated validator-set transition propagation,
weak-subjectivity history gates, mandatory certified-committee enforcement, and
fail-closed npm/PyPI authority sidecars. Focused non-Rust evidence is green:
capability matrix 18/18, CR-07 activation 11/11, weak-subjectivity publication
11/11, adapter contract 25 passed/1 skipped, npm verifier 20 passed/1 skipped,
and PyPI verifier 29 passed/1 skipped. The focused node E2E binary passes 8/8
(health, admission, verification, duplicate rejection, SSE, metrics, and
signature rejection), and the node/CLI unit suites are green after the sidecar
process-containment hardening. The full workspace command compiled all targets
but its wrapper timed out before emitting an aggregate test summary; that result
remains explicitly uncaptured.

The fresh-genesis candidate ceremony completed locally on 2026-08-01. The
rotated spec-signing key and fresh validator/service keys are stored in the
CREG Google Secret Manager project. The four-candidate signed spec validates
with genesis
`0xbca931fadf9ff600c5db30fefa63e709f3ae0d01d65520a407c0b1ad18bd1ec5`, and the
spec-server mirror matches the canonical spec and detached signature. Public
metadata and secret IDs are recorded in
[`fresh-genesis-ceremony-2026-08-01.json`](../chain-registry/testnet/archive/fresh-genesis-ceremony-2026-08-01.json).

Public activation remains blocked until the full-workspace aggregate result,
independent custody acknowledgements for the four candidate validators,
independently controlled bootnodes,
positive production authority fixtures/trust approval, and live
replay/partition/reorg/rebuild/long-range/process-kill rehearsals are
published. No mandatory public mode should be enabled from the repository
fixtures alone.

The repository-disclosed development key has been removed from the tracked lab
helper. All 22 local quarantine copies were deleted; the non-secret deletion
record is [`quarantine-deletion-2026-08-01.json`](../chain-registry/testnet/archive/quarantine-deletion-2026-08-01.json).
The tracked npm/Sigstore material remains `candidate_unapproved`. The non-secret retirement record is
[`development-key-retirement-2026-08-01.json`](../chain-registry/testnet/archive/development-key-retirement-2026-08-01.json).
The repository-side ceremony and rehearsal preflight is archived at
[`testnet/archive/operator-readiness-preflight-2026-08-01.json`](../chain-registry/testnet/archive/operator-readiness-preflight-2026-08-01.json);
it records blocked gates and is not activation evidence.

### Release packaging status

The repository now contains a release workflow at
[`chain-registry/.github/workflows/release-binaries.yml`](../chain-registry/.github/workflows/release-binaries.yml).
It builds pinned-signing CLI/node archives for Linux, macOS, and Windows,
compares deterministic Linux builds, publishes SHA-256 manifests, signs the
manifest with Sigstore, and emits GitHub Artifact Attestations. The Windows
and Unix installers verify the matching archive hash before extraction. No new
release tag has been published from this workflow yet; the existing June
artifacts are not valid evidence for the fresh-genesis candidate.

---

## What “paused fleet” means

- Public REST at `https://api.testnet.cregnet.dev` may be **unreachable** while GCP hosting is stopped.
- Publishers and integrators should **not** assume the public API is online without checking health first.
- Operators can still run nodes locally or on their own infrastructure using [PUBLIC_TESTNET_QUICKSTART.md](./PUBLIC_TESTNET_QUICKSTART.md) and [TESTNET_SEPOLIA_RUNBOOK.md](./TESTNET_SEPOLIA_RUNBOOK.md).

---

## Resume public hosting

When ready to bring the Sepolia fleet back:

```powershell
cd chain-registry\testnet\gcp
.\resume-testnet-hosting.ps1
```

See also [WHAT_TO_DO_NEXT.md](./WHAT_TO_DO_NEXT.md) Track A (system verification before resume).

---

## Related documents

| Document | Purpose |
|----------|---------|
| [TESTNET_PHASE_SCOPE.md](./TESTNET_PHASE_SCOPE.md) | Alpha limits, package statuses, committee semantics |
| [PUBLIC_TESTNET_QUICKSTART.md](./PUBLIC_TESTNET_QUICKSTART.md) | Publisher / validator onboarding |
| [WHAT_TO_DO_NEXT.md](./WHAT_TO_DO_NEXT.md) | Maintainer planning hub |
