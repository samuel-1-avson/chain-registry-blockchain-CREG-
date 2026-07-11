# ADR-015: Ecosystem integration strategy

- **Context/problem:** cross-registry breadth increases adapter and UX complexity.
- **Options:** all ecosystems at once; central replacement registry; one native integration at a time.
- **Decision:** complete npm and PyPI exact-byte integrations first, each with a conformance corpus, then expand one ecosystem at a time.
- **Security implications:** namespace, lockfile, provenance, and rollback semantics are tested per ecosystem.
- **Operational implications:** each adapter has an owner, SLO, rate limits, and incident runbook.
- **Migration implications:** existing shims remain compatibility surfaces until native integrations pass gates.
- **Rejected alternatives:** ambiguous `latest` and silent unqualified fallback.
- **Revisit conditions:** measured adoption and adapter maintenance capacity.
