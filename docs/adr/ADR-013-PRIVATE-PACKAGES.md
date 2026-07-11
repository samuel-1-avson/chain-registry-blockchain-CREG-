# ADR-013: Private enterprise package architecture

- **Context/problem:** the current shielded path has a single-node X25519 trust boundary and public-storage risk.
- **Options:** public IPFS encryption; threshold encryption immediately; tenant-isolated KMS/workers and private object domains.
- **Decision:** keep private-package promises disabled until tenant isolation, encryption, retention, and deletion are proven.
- **Security implications:** no plaintext private code enters public immutable state.
- **Operational implications:** tenant-controlled keys and regional policies are explicit costs.
- **Migration implications:** private legacy data requires rights/retention review.
- **Rejected alternatives:** labeling single-node encryption as threshold privacy.
- **Revisit conditions:** audited tenant KMS/worker and deletion design.
