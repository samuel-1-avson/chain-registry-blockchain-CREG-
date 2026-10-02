# R7 blocker repair — 2026-10-02

## Completed this run

- Captured all eight real nodes before recovery. All agree on signed R7, finalized height 20, 21 stored blocks and tip `730b01c1a67a4796a16f599169e4ebf77e01d2e8ce92d1e4fb3715a7c4354285`. Recovery queues and unresolved PBFT rounds are clear.
- Recovered V4's stalled L1 sync with one guarded restart of its existing container. Its last poll had remained at 2026-09-29 14:03:48 UTC. After restart, sync was `synced`, cursor and finalized source block were 11,827,708, and the poll timestamp was 2026-10-02 08:11:28 UTC.
- Verified identical container/image IDs, persisted data mount, P2P identity hash, environment hashes, h20 hash and retained recovery counts before/after V4 restart. The signing journal remained present. No chain reset, image replacement or package submission occurred.
- Captured the complete fleet again at 08:14:10 UTC. All four validators now have current sync cursors. Every validator still has only one configured RPC provider. The observer readiness checks pass; fleet image IDs remain different.
- Prepared genuine `npm:has-flag@4.0.0`: verified registry SHA512/SHA1, reviewed five regular archive files, no install lifecycle hooks or runtime dependencies, and captured a real OSV response with zero advisories. All eight nodes returned package absence at the shared h20. The current scanner profile does not yet include this candidate.
- Repaired direct sponsored-request policy enforcement: disabled policy rejects before RPC work/state reservation, actual transaction gas is estimated before reservation, and submission uses an explicit policy ceiling. Scoped relayer tests passed 4/4; targeted formatting and diff checks passed. These changes are source repairs, not deployed economic acceptance.
- Prepared a build from an explicit committed source archive using Rust 1.93.0, locked Cargo dependencies, Phase C features and the accepted signing public key. Builder/runtime images are pinned by digest, source hashes are recorded, and the dirty working tree is excluded.

## Remaining gates

| Gate | Current state | Next action |
| --- | --- | --- |
| RPC resilience | Missing two provisioned independent provider secrets | Create Alchemy/Infura Ethereum Sepolia URL secrets, verify quota and header/log agreement, then configure one validator at a time. |
| V4 freshness | Recovered on current older image | Deploy the tested sync supervision repair with the candidate image; verify continued fresh polling and restart recovery. |
| Deployment consistency | Five differing image IDs across eight nodes | Build and verify the committed candidate, stage on an observer, then use guarded canary/validator rollouts with preserved data/signing state. |
| Fresh acceptance | Real artifact and eight-node absence captured; not published | Extend the immutable scanner snapshot, capture readiness and absence again, attach clients, sign/publish once, verify three distinct validator approvals and certified matching finality across all eight. |
| Economic acceptance | Policy guards repaired; execution/recovery remains incomplete | Finish relayer transaction journaling and finalized receipt/accounting verification; implement reviewed reward contract and canonical-finality outbox/authorized dispatcher before real transactions. |

The laptop had approximately 12 GiB free, insufficient for the documented cold native build allowance. Build preparation therefore targets the project's existing Cloud Build service, separate from the real validators. Preparation alone is not a successful build or rollout.

## Economic review

The faucet already has real funding, claim, reconciliation and restart evidence. That evidence does not establish rewards or sponsored staking.

Sponsored staking still needs a journal of the exact signed transaction and deterministic hash before broadcast, reserved signer nonce/intent binding, recurring recovery of ambiguous submissions, canonical finalized receipt checks, the exact staking event, and token/stake balance deltas. Duplicate/restart acceptance must prove one durable stake change.

Reward dispatch is intentionally skipped before canonical finality. There is no reward outbox or dispatcher yet. The current contract permits Governance/Staking only, has no finalization replay key, and needs participant uniqueness/bounds, vote-correctness points, bounded reputation and fully funded accounting repairs. Review the contract/dispatch change and exact authorized amounts before executing rewards. A node key alone is not sufficient authorization.

## Evidence

- `fleet-readiness-before.json` — live eight-node baseline; V1 state inferred from live health in this capture.
- `v4-sync-recovery.json` — guarded restart and exact preserved-state verification.
- `fleet-readiness-after-v4.json` — fresh complete capture including actual runtime image IDs/provider hostnames.
- `fresh-package/PREPARATION-REVIEW.md` — genuine artifact review and eight-node absence evidence.
- Build records/manifests are recorded separately; check their status before treating an image as built.

No reward or sponsored-stake transaction was sent. RPC URLs and signing secrets are excluded from this evidence.

## Creating the missing RPC secrets

1. [Alchemy](https://www.alchemy.com/docs/create-an-api-key): create an app, choose Ethereum Sepolia, and copy its HTTPS endpoint from Endpoints.
2. [Infura](https://docs.infura.io/get-started/infura/): open My First Key / All Endpoints and copy the Ethereum Sepolia HTTPS endpoint.
3. In [GCP Secret Manager](https://console.cloud.google.com/security/secret-manager?project=fiffi-500411), create `creg-sepolia-rpc-alchemy` and `creg-sepolia-rpc-infura`, storing the corresponding full URLs as secret values. [Google instructions](https://docs.cloud.google.com/secret-manager/docs/creating-and-accessing-secrets).
4. Share secret names only. Endpoint correctness, access permissions and available provider capacity still require verification before rollout.
