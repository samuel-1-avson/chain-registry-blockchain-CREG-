# STRIDE threat model

| Threat | Boundary | Control | Test |
|---|---|---|---|
| Publisher spoof/namespace takeover | Gateway/ownership | Dual proof, upstream authority, delegation history | Wrong-key, wrong-domain, transfer, recovery |
| Malicious package/sandbox escape | Worker | Disposable isolated workers, no ambient credentials, resource limits | Malicious corpus, escape, decompression bomb |
| Evidence fabrication/collusion | Evidence/quorum | Typed signed envelopes, producer/domain diversity, profile quorum | Forged signature, duplicate, cartel/common-mode |
| Split-view/rollback | Log/client | Merkle consistency, witnesses, expiry, TUF roles | Split-view, freeze, rollback, stale revocation |
| Build compromise | Provenance/build | SLSA/in-toto, independent rebuilds, toolchain digests | Nonreproducible and altered-build fixtures |
| Storage withholding | Object/evidence store | Multi-domain replication, probes, expiry | Provider loss and restore drill |
| Governance/treasury theft | Settlement/admin | Separate roles, unique signers, timelocks, hardware keys | Fuzz/invariant/pause recovery |
| Privacy disclosure | Private package path | Tenant encryption/isolation, no public plaintext | Retention/deletion/access audit |

Residual risks are displayed with each qualification. This model is living and
must be updated after incidents, new integrations, or profile changes.
