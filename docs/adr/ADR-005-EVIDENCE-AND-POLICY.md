# ADR-005: Evidence envelope and policy model

- **Context/problem:** local scanner/environment outputs were treated as identical consensus evidence.
- **Options:** raw vote equality; one central scanner; typed signed envelopes and versioned profiles.
- **Decision:** quorum agrees on profile/disposition while preserving distinct evidence objects.
- **Security implications:** evidence remains independently auditable; unavailable tools cannot pass.
- **Operational implications:** tool images, datasets, profiles, and retention need versioning.
- **Migration implications:** old digests remain historical but cannot satisfy new profiles without required predicates.
- **Rejected alternatives:** LLM or score as safety proof.
- **Revisit conditions:** evidence schema or profile changes require conformance vectors.
