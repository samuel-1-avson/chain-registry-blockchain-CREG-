# ADR-012: Governance and emergency control separation

- **Context/problem:** pause could block revocation/unpause proposal; signer/cancellation rules were weak.
- **Options:** one admin; governance-only emergency; separated security council, timelock, and scoped pause escape.
- **Decision:** unique signers, timelocks, separate treasury/security/dispute powers, and paused unpause/revocation paths.
- **Security implications:** containment remains possible without granting upgrade/treasury authority.
- **Operational implications:** drills and retrospective ratification are required.
- **Migration implications:** existing Governance deployments need a reviewed upgrade or replacement.
- **Rejected alternatives:** AI as final dispute authority.
- **Revisit conditions:** legal structure and signer independence mature.
