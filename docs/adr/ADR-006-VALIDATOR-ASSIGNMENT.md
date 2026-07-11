# ADR-006: Validator assignment and independence

- **Context/problem:** committee recording and analyzer sampling diverged above 50 validators.
- **Options:** two independent selectors; all validators; one snapshot-bound ranked committee with expansion.
- **Decision:** persist one deterministic assignment derived from subject, epoch, set snapshot, capability, and diversity rules.
- **Security implications:** required quorum is reachable and assignments are auditable.
- **Operational implications:** dropout/abstention expands deterministically and exposes control-domain concentration.
- **Migration implications:** legacy committee fields are read-only evidence.
- **Rejected alternatives:** performance-only selection that centralizes mandatory work.
- **Revisit conditions:** VRF/epoch proof is independently reviewed.
