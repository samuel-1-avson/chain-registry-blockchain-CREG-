# Remediation implementation status

**As of:** 2026-07-29

**Release posture:** research alpha only; no mainnet, production-security, absolute-safety, or validity-rollup claim.

## Phase 0 status

| Area | Status | Evidence |
|---|---|---|
| ZK package qualification | Disabled in source/config | Registry entry points revert; CLI no longer creates publisher ZK claims. The workflow now states accurately that the existing circuit binds content and manifest limbs but does not qualify package safety. |
| Legacy batch checkpoint | Disabled by default | Bridge requires explicit development flags; Registry entry point reverts |
| gRPC admission | Hardened | Uses the same publisher-authenticated admission policy as REST |
| Publisher proof | Test incomplete | Versioned v2/v3 dual-domain payload, CLI EVM signing, durable nonce replay protection, and signed gRPC issued-at transport are implemented; upstream authority remains |
| Namespace ownership | Implementing; activation disabled | Chain/pending owner checks, signed delegate/transfer/recovery lifecycle, durable nonces, restart/rebuild, and quorum certificates are implemented. Cryptographic npm and PyPI verifier candidates now exist, but npm lacks an official signal proving trusted-publisher authentication mode; PyPI still requires the node private-temp launch contract; both require reviewed production policy/trust material and live positive fixtures. |
| Authority evidence archive | Implemented; operational evidence pending | Evidence and official artifacts share bounded byte/entry quotas, minimum-retention pressure GC, atomic pair admission, corruption/symlink rejection, and stale-temp cleanup. Current locking is process-local; shared-filesystem coordination, capacity alerts, backups, and legal-hold policy remain operator work. |
| Committee assignment | Repository tests passed; activation disabled | Pipeline reuses one snapshot-certified assignment bound to fork anchor and anchor-derived randomness. Mandatory mode remains gated on a real independently verified checkpoint and live multi-operator rehearsal. |
| Historical validator authority | Repository tests passed; live rehearsal pending | Quorum-certified transitions, independent gossip validation, peer history retrieval, weak-subjectivity checkpoint ancestry, retention, and reorg truncation are implemented. A real checkpoint and live partition/reorg/rebuild/long-range/process-kill evidence do not yet exist. |
| Signed Sepolia specification | Blocked; migration required | The canonical spec currently declares a genesis hash that does not match the computed payload, both static-server mirrors differ from the canonical bytes/signature, and its signing public key corresponds to repository-disclosed development key material. Signing scripts now remove literal private keys, require an environment-protected secret by name, reject the disclosed public key, and refuse key/spec mismatch; no replacement spec or signature was fabricated. |
| Evidence quorum | Implementing | Distinct worker evidence may coexist under one profile/disposition |
| Publisher liability | Implementing | Staking reserves per-release bond; contract tests added |
| Contract governance and verdict signatures | Implementing | First-pause/cooldown behavior and fuzz-test defects are fixed. Epoch-bound EIP-712 validator verdict/admission signatures and same-size-set replay tests are in active review. |
| Pause recovery | Implementing | Paused unpause proposal and emergency revocation paths added; first-pause cooldown regression is fixed |
| Canonical event history | Design approved | Target is witnessed Merkle history; legacy local chain remains migration input |
| Client freshness | Not started | TUF-style metadata and witnessed inclusion remain Phase 4 |
| Durable orchestration | Test incomplete | Pending work and finalized transactions have durable idempotency, bounded hydration, retry/backoff, DLQ, and canonical acknowledgement; process-kill rehearsal remains |

## Verification commands

| Command | Result | Interpretation |
|---|---|---|
| `npx --yes @mermaid-js/mermaid-cli --version` | Passed (`11.16.0`) | Mermaid source rendered successfully |
| Mermaid render of the audit | Passed; 18 SVGs | Report diagrams are portable image assets |
| `cargo check --locked -p chain-registry-node --lib -j1` | Passed | Node library compiles after the authority archive work |
| Five focused checkpoint-history tests | Passed (5/5) | Checkpoint ancestry and history gates execute successfully |
| Authority-adapter and lifecycle/API filters | Passed (10/10 and 6/6) | Existing adapter boundary and authenticated lifecycle routes execute successfully |
| Authority archive tests | Passed (7/7 on Windows) | Quota, retention/GC, atomicity, corruption, and cleanup cases pass; the Unix-only symlink case was not exercised on Windows |
| Complete node suite | Passed (209/209) | Two stale test expectations were corrected and the rerun has zero failures |
| CLI suite | Passed (41/41, plus five zero-test targets) | CLI tests execute successfully |
| Full Rust workspace/all-target suite | Failed after partial green execution | CLI 41/41, node library 209/209, and node main 211/211 passed; node E2E passed 4/8 and exposed stale Ed25519-only fixtures. Cargo therefore did not run later workspace targets. Fixtures are being upgraded to valid dual-domain proofs before a full rerun. |
| `cargo fmt --all -- --check` | Pending | The pre-format check found format-only diffs; formatting is deferred until the active all-target run finishes |
| Checkpoint and CR-07 activation validators | Passed (11/11 each) | Disabled-by-default manifests and certificate validation fail closed |
| npm verifier | Passed (20/20; 1 external production fixture skipped) | Offline registry/provenance cryptography and archive/challenge checks pass; the verifier deliberately cannot return success without an official npm authentication-mode signal |
| PyPI verifier | Passed (29/29; 1 external production fixture skipped) | Offline PEP 740/Sigstore verification and bounded archive parsing pass; the current Rust cleared environment cannot yet launch the frozen Windows candidate |
| Local Foundry v1.6 nightly suite | Passed (111/111 across 12 suites) | Contract tests, including 1,000-run fuzz cases, pass locally; the pinned v1.7.1 clean-CI run is still pending |
| Contract CI workflow and dependency bootstrap smoke | Passed locally | Workflow pins Foundry/action/dependency revisions; YAML, shell syntax, and clean dependency clone were validated, but no remote CI run is claimed |
| `creg chain-spec validate testnet/chain-spec.sepolia.json` | Failed closed | Declared genesis `0x0bb4…6660` differs from computed `0xddd1…1e0a`; canonical and two spec-server SHA-256 values also differ. This is a real publication blocker, not a test defect. |
| Workflow/whitepaper PDFs and manifests | Stale | Markdown changed after the current PDFs; regeneration, mirror synchronization, hash refresh, and all-page visual verification remain required |

## Required next work

1. Finish the current full Rust workspace run, add the private sidecar scratch-directory contract, then rerun rustfmt/check and affected node/workspace tests.
2. Finish epoch-bound EIP-712 verdict/admission review and run the pinned Foundry v1.7.1 suite.
3. Add reviewed production npm/PyPI policy and trust material, an official npm authentication-mode signal, live positive fixtures, platform artifacts, and signed activation approval; keep recovery fail closed until every gate passes.
4. Regenerate and visually verify the workflow/whitepaper PDFs, synchronize public and Functions mirrors, and refresh all manifest hashes.
5. Run a witnessed key/spec migration: select the intended genesis/topology, rotate the disclosed signing/validator material, recompute and sign the exact canonical spec, independently verify it, then copy exact bytes/signature to both static-server names. Do not publish the current artifacts.
6. Publish an independently verified validator-set checkpoint and complete live multi-operator replay, partition, reorg, rebuilt-node, long-range, and process-kill rehearsals before mandatory rollout.
7. Recruit independently controlled validators and bootnodes; the recorded one-validator topology has no Byzantine fault tolerance.
8. Complete SEC-401, contract/ZK/sandbox/protocol reviews, deployment migrations, operator rehearsal, and independent security sign-off.
9. Continue witnessed-log/client freshness, resource quota, SBOM/provenance, availability, governance API, web/E2E CI, and derived-index completeness work.
