# Policy profiles

Profiles are versioned, deterministic documents. They define required and
optional evidence, hard blockers, freshness, allowed producers/builders,
minimum witnesses, expiry, dispute behavior, and client action.

| Profile | Minimum use |
|---|---|
| Baseline v1 | Identity, ownership, digest, provenance where available, dependency/SBOM, secrets/license, static, and current vulnerability snapshot |
| Sandboxed v1 | Baseline plus hermetic behavioral sandbox with declared network/filesystem/process policy |
| Reproducible v1 | Sandboxed plus two independent hermetic builds matching the named recipe/toolchain |
| Critical Package v1 | Reproducible, independent reviewers, stronger freshness, witness threshold, and mandatory dispute response |
| Enterprise Custom | Tenant-owned requirements and retention/encryption policy; never weaker than baseline hard blockers |

The UI exposes evidence completeness, producer independence, provenance
strength, reproducibility, freshness, and residual risks separately. A score
is not a probability that the package is safe.
