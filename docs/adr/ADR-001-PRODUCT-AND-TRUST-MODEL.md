# ADR-001: Product and trust model

- **Context/problem:** A package verifier cannot prove absolute safety, while the current system overloads `Verified`.
- **Options:** safety oracle; custom L1; evidence/policy service.
- **Decision:** CREG is an evidence-first service with bounded claims and explicit residual risk.
- **Security implications:** scanners, builders, policy, witnesses, and clients have separate trust statements.
- **Operational implications:** profiles, evidence retention, and freshness are first-class SLOs.
- **Migration implications:** rename/remove absolute-safety UI and derive claims from events.
- **Rejected alternatives:** “safe to install,” malware-free, and token-first positioning.
- **Revisit conditions:** measured demand for a stronger shared ordering property.
