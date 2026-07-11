# ADR-011: Storage and evidence retention

- **Context/problem:** memory/file queues and single gateways can drop or withhold evidence.
- **Options:** local files; one object provider; transactional queue plus multi-domain immutable objects.
- **Decision:** durable idempotent jobs, dead letters, replication, integrity probes, retention classes, and restore tests.
- **Security implications:** no-drop and availability expiry are auditable controls.
- **Operational implications:** storage cost is metered per profile and retention tier.
- **Migration implications:** export pending state and verify every object digest.
- **Rejected alternatives:** CID alone as availability guarantee.
- **Revisit conditions:** SLO or jurisdictional retention changes.
