# CREG Master Remediation Plan

**Status:** active — Phase 0 safety freeze

**Primary baseline:** [comprehensive audit](../CREG_COMPREHENSIVE_AUDIT_2026-07-10.md)

## Scope and decision

This program concerns only the core CREG package-verification and blockchain
system. The product is being redirected from an experimental custom Layer 1 to
an evidence-first software supply-chain verification service. No production or
mainnet-security claim is permitted until the acceptance gates are met.

The target is a hybrid system: a durable submission and verification service,
typed signed evidence, a canonical append-only transparency history with
independent witnesses, exact-byte client verification, and narrowly scoped L2
checkpoint/bond contracts. The current custom PBFT chain and package-safety ZK
paths are not a production target.

## Phase 0 — Baseline and safety freeze

| Work item | Status | Evidence / result |
|---|---|---|
| Read and normalize the audit baseline | Complete | Audit findings CR-01 through LO-30 are registered in the traceability matrix. |
| Render audit diagrams portably | Complete | The rendered audit embeds 18 generated SVG diagrams. |
| Disable ZK package qualification in new code | Implemented; test execution pending | `Registry.sol`, CLI publishing, and gRPC admission no longer use a publisher ZK proof as a qualification path. |
| Disable unsupported legacy checkpointing by default | Implemented; test execution pending | The node bridge exits unless two explicit development-only environment flags are set; the replacement contract function reverts. |
| Freeze unsafe public claims | In progress | Capability matrix records the research-alpha posture and prohibited terminology. Deployed surfaces still require operator action. |
| Record deployed-state and credential limits | Complete | This repository review has no authority over deployed contracts, remote config, cloud storage, or secret rotation; required operator actions are listed below. |
| Create traceability and status records | Complete | This document, the traceability matrix, implementation status, and capability matrix are the Phase 0 baseline. |

### Required operator actions for existing deployments

Source changes do not alter already deployed EVM bytecode, a remotely served
signed chain specification, or a running relayer. Before exposing any existing
environment, an authorized operator must:

1. Submit and execute `setZKValidationEnabled(false)` on every legacy Registry.
2. Stop the legacy bridge/relayer and remove any job or UI action that invokes
   `submitRollupBatch`.
3. Remove `safe`, `fully verified`, `validity rollup`, and decentralization/BFT
   claims from public deployment configuration and user interfaces.
4. Publish a signed capability notice stating that the environment is a
   coordinated research alpha, including its validator and witness control
   domains.
5. Back up state, retention metadata, contract addresses, deployment
   transactions, signer inventories, and encrypted secret references before
   migration. Do not copy secrets into this repository.
6. Rotate any credential that has been exposed or whose custody cannot be
   demonstrated. This requires the responsible operator; no credential was
   accessed or rotated by this remediation work.

## Ordered implementation plan

| Phase | Objective | Required exit gate |
|---|---|---|
| 0 | Safety freeze, baseline, capability truthfulness, traceability | Unsafe ZK/checkpoint paths disabled; all findings have a disposition. |
| 1 | Normative architecture, ADRs, schemas, threat model, migration design | Identity, ownership, evidence, policy, log, and client semantics have test vectors. |
| 2 | Publisher identity, namespace ownership, replay protection, event-derived claims | No publisher can claim another stake or namespace; state derives from immutable events. |
| 3 | Typed evidence, durable orchestration, isolated workers, real SBOM/provenance | Qualification is explainable; missing evidence never passes. |
| 4 | Transparency log, witnesses, freshness metadata, exact-byte clients | Clients verify subject, inclusion, consistency, freshness, and revocation. |
| 5 | Minimal contracts and recoverable governance | Only justified contract functions remain; invariant/fuzz tests and audit gates pass. |
| 6 | Cost metering, payments/bonds, backup/restore, incident drills | Economics work without token-price assumptions; recovery has been exercised. |
| 7 | Independent test network | Four independent witnesses, seven independent reviewers, adversarial exercises, and external reviews complete. |
| 8 | Production candidate | Every Critical/High finding is closed or formally accepted by authorized review. |

## Non-negotiable rules

- A proof, signature, scan, or consensus vote may claim only what its formal
  statement and evidence establish.
- `Verified` is not a synonym for safe. Consumers receive explicit claims,
  profile/version, evidence freshness, and residual-risk information.
- The canonical event history is the source of truth; databases, search,
  explorers, and caches are rebuildable views.
- Evidence, not a binary verdict or token balance, is the primary product.
- No legacy feature is re-enabled without an ADR, test vectors, adversarial
  tests, configuration gate, independent review, and a documented operator
  rollout.

## Phase 0 residual risks

- Existing deployed Registry contracts retain their historical bytecode until
  authorized governance deploys/replaces or disables them.
- The bridge's legacy code remains available only behind explicit development
  flags for research; it is not a production checkpoint protocol.
- Publisher identity, namespace ownership, liability locking, pause recovery,
  committee authority, and freshness protection remain incomplete Phase 1–5
  work; implemented controls and their residual rollout dependencies are kept
  current in the master tracker.
- Rust and Arena regression suites now have captured local passing evidence.
  Foundry verification and independent review remain outstanding, and no
  finding is marked fixed solely from a working-tree test result.

## Required documentation set

The normative target, trust boundaries, data flow, protocol schemas, profiles,
log/client rules, threat model, key management, incident response, economics,
governance/disputes, testing gates, deployment/DR, and migration plan are
maintained in the sibling `architecture/`, `protocol/`, `security/`,
`economics/`, `governance/`, `testing/`, `operations/`, and `migrations/`
directories. Finding-level status is in
[`AUDIT_FINDINGS_TRACEABILITY.md`](AUDIT_FINDINGS_TRACEABILITY.md).
