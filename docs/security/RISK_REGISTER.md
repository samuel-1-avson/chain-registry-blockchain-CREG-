# Security risk register

The authoritative finding-by-finding register is
[`../remediation/AUDIT_FINDINGS_TRACEABILITY.md`](../remediation/AUDIT_FINDINGS_TRACEABILITY.md).
This document is the operational view:

| Risk class | Current control | Required gate |
|---|---|---|
| Identity/ownership | Versioned payload and namespace guard are implementing | Strict dual proof, persistent nonce, upstream/delegation conformance |
| Unsafe qualification | ZK and legacy batch paths frozen | No unsupported claims; regression tests and deployed-state action |
| Consensus | Research-only posture | Witnessed log or independently reviewed mature consensus |
| Evidence | Typed profile/disposition quorum implementing | Reproducible envelopes and malicious corpus efficacy |
| Client | Exact-byte hashing exists | Freshness, inclusion, consistency, revocation, rollback tests |
| Contracts | Liability/pause changes implementing | Pinned Forge, fuzz/invariants, Slither, external audit |
| Operations | Monitoring foundations exist | Durable queues, restore tests, drills, SLO history |
