# Remediation decision log

| Date | Decision | Rationale | Consequence |
|---|---|---|---|
| 2026-07-10 | Evidence and deterministic policy are the product trust primitive. | Consensus alone cannot establish package safety. | See ADR-001/002/005. |
| 2026-07-10 | Retire ZK-only package qualification and false batch validity claims. | The circuit statements do not bind scanner execution or a state transition. | Registry paths revert; ZK flag is off in shipped specs. |
| 2026-07-10 | Do not advance custom PBFT as a production mainnet. | Safety/liveness gaps and single-operator topology remain. | Treat it as research/compatibility only. |
| 2026-07-10 | No native tradable token is required for MVP. | Verification must not depend on volatile speculation. | Stable-value cost accounting is future work. |
| 2026-07-14 | Add a repository security-freeze CI guard. | Remote configs and deployed bytecode are external, but checked-in regression must fail closed. | Every shipped chain spec is checked for `zk_validation:true`; Registry retirement guard is checked. |

Open decisions requiring evidence: ownership authority adapters, durable queue
technology, log/witness implementation, freshness metadata, contract surface,
and production topology. No external deployment, audit, or legal conclusion
is inferred from this log.
