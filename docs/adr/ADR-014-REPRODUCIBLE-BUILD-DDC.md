# ADR-014: Reproducible-build and DDC policy

- **Context/problem:** release-assurance scripts describe CREG binaries, not submitted packages; a no-target DDC run is not evidence.
- **Options:** documentation-only; run every package through DDC; risk-based SLSA/in-toto provenance and independent rebuilds.
- **Decision:** integrate provenance first; use reproducible rebuilds for risk tiers; report DDC not-applicable when no compiler-like target exists.
- **Security implications:** source/build/artifact mismatch becomes detectable without claiming universal safety.
- **Operational implications:** hermetic builders, toolchain images, and independent builder domains are required.
- **Migration implications:** legacy package records lack this claim and remain incomplete.
- **Rejected alternatives:** pseudo-SBOM or empty DDC inventory as compliance proof.
- **Revisit conditions:** ecosystem conformance corpus demonstrates feasible coverage.
- **In-repo slice (2026-08-23):** first-party `Cargo.lock` inventory and
  lockfile-bound provenance (`creg supply-chain check`, `creg sbom --workspace`)
  exist and must not be labeled SLSA. Submitted-package provenance and
  independent builders remain operational.
