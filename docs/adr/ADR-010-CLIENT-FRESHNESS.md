# ADR-010: Client freshness and rollback protection

- **Context/problem:** a client can receive stale qualified metadata after revocation.
- **Options:** cache trust; live RPC only; TUF-style expiry plus witnessed log inclusion/consistency.
- **Decision:** use pinned root/snapshot/timestamp/targets semantics and freshness checks for every install decision.
- **Security implications:** freeze, rollback, mix-and-match, stale-revocation, and split-view failures become explicit.
- **Operational implications:** key rotation and emergency freshness forcing are runbooks.
- **Migration implications:** clients must migrate before public testnet claims.
- **Rejected alternatives:** DNS/TLS or local cache as sole trust root.
- **Revisit conditions:** a reviewed equivalent protocol replaces TUF roles.
- **In-repo slice (2026-08-23):** install/shim admission persists an
  anti-rollback journal, expires/skew-checks metadata, and requires
  `verify_witnessed_inclusion` when log/witness keys are configured. Unsigned
  node heads stay monitoring-only. `creg witness run` and log `signed-head`
  exist in-repo; independent witness operators, a split-view drill, and a
  threshold timestamp role remain operational.
