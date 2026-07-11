# ADR-009: ZK feature disposition

- **Context/problem:** package circuit proved caller-selected score/sandbox assertions; batch circuit proved no state transition.
- **Options:** keep fast path; formalize/audit circuits; disable security decisions and retain research fixtures.
- **Decision:** disable both production qualification/checkpoint paths; remove misleading claims.
- **Security implications:** no vacuous proof can create a qualification or validity claim.
- **Operational implications:** no proving-key dependency in admission.
- **Migration implications:** deployed contracts require governance disable/migration action.
- **Rejected alternatives:** “proof exists” as evidence of scanner execution.
- **Revisit conditions:** formal statement, witness binding, transition/DA proof, audit, and adversarial tests.
