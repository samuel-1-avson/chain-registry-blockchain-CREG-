# ADR-003: Transparency log versus custom L1

- **Context/problem:** Custom PBFT and ZK checkpoint claims add a large unaudited trusted base.
- **Options:** continue PBFT; adopt mature appchain; witnessed log plus L2 roots.
- **Decision:** use witnessed log and minimal L2 anchoring for the target; keep PBFT research-only.
- **Security implications:** L2 anchors ordering/bond events, not package safety; witnesses detect split view.
- **Operational implications:** simpler upgrades and lower idle consensus cost.
- **Migration implications:** legacy blocks become read-only import evidence.
- **Rejected alternatives:** mainnet claims from a one-validator chain or vacuous batch proof.
- **Revisit conditions:** demonstrated log limitation and independently reviewed CometBFT design.
