# Key management

Use separate hardware/KMS-backed keys for publisher identity, validator
evidence, log append, witness, checkpoint relay, governance, treasury, and
deployment. Workload identities receive short-lived scoped credentials; worker
containers receive no governance or treasury secrets.

Key ceremonies record participants, quorum, device attestation, backup
location, rotation interval, and revocation procedure. Signatures include
domain, chain/contract, epoch, subject, nonce, and expiry. Compromise response
rotates the affected root, publishes a checkpoint/revocation event, invalidates
old metadata, and forces client freshness.

Secrets must not be copied into the repository, logs, fixtures, or public IPFS.

## CR-07 signed activation gate

`chain-registry/config/cr07-activation-manifest.json` is the
repository-controlled authorization record for authoritative npm/PyPI
namespace ownership. It does not replace the chain-spec signature or create a
trust root. It prevents an operational chain spec from acquiring CR-07
consensus fields without an explicit, reviewable rollout record.

The normative artifact, policy, trust, ceremony, rollout, rollback, and
acceptance requirements are defined in
[`protocol/NAMESPACE_AUTHORITY_ACTIVATION.md`](../protocol/NAMESPACE_AUTHORITY_ACTIVATION.md).

The current manifest is intentionally `disabled`. In that state, CI requires
every path in `guarded_chain_specs` to omit
`namespace_authority_activation_height` and to have no
`namespace_authority_policies`. No production verifier, policy, trust root, or
signature is represented by the disabled manifest.

The historical local/Sepolia fixture signing key is listed in
`disallowed_signing_pubkeys` because its private material has appeared in
repository testnet tooling. It is not an acceptable CR-07 activation trust
root. A future activation must use a newly generated, independently controlled
signing key and a reviewed signing ceremony.

Changing the manifest to `signed` fails closed unless the manifest and selected
chain spec agree on:

- manifest version, chain-spec version, and `creg-chain-spec-v1` signature
  domain;
- chain ID, non-placeholder genesis hash, activation height, and a
  non-disallowed Ed25519 signing public key;
- at least one pinned `npm_trusted_publisher` policy and one pinned
  `pypi_trusted_publisher` policy;
- canonical adapter ID/version and non-zero SHA-256 pins for each verifier
  executable, policy bundle, and trust-root bundle;
- computed genesis hash, canonical full-spec SHA-256, and
  `creg-cr07-activation-v1` payload SHA-256;
- a valid detached Ed25519 signature over the canonical full chain spec; and
- canonically identical content and identical signatures at every declared
  repository distribution mirror.

Any other guarded chain spec must remain unactivated. Unknown manifest fields
or versions, unsafe JSON numbers, path traversal, duplicate adapter identities,
missing ecosystem policies, placeholder hashes, mirror conflicts, and forged
signatures are rejected.

### Activation procedure

1. Complete independent review of the real npm and PyPI self-contained
   verifier executables, official trust material, and policy bundles. The
   fail-closed rejection fixture and generated Python PyPI console launcher are
   not production verifiers; the latter's hash does not cover imported code or
   dependencies. The locked PyInstaller recipe can produce a byte-reproducible
   same-target candidate, but every platform/architecture/CPython build still
   needs independent reproduction and review. PyPI also remains disabled until
   the Rust launcher provides a fresh private per-invocation scratch directory
   through only `TEMP`, `TMP`, and `TMPDIR` in its otherwise-cleared
   environment.
2. Hash the exact immutable complete executable, policy, and trust-root bytes
   and add those pins plus a coordinated future height to a new candidate chain
   spec. A directory/image verifier requires a Rust/config bundle-pinning
   extension before activation.
3. Recompute and embed the chain-spec genesis hash.
4. From `chain-registry/`, inspect the candidate values:

   ```bash
   node scripts/validate-cr07-activation.mjs \
     --inspect-spec testnet/chain-spec.cr07-candidate.json
   ```

   Inspection only computes candidate pins. It does not verify a signature or
   authorize rollout.
5. Sign the exact candidate using a new offline/independently controlled key.
   Never commit private signing material. Record the public-key ceremony,
   reviewer approvals, and operator rehearsal outside this manifest.
6. Copy the exact signed spec and signature to each repository mirror, add all
   paths to `guarded_chain_specs`, and fill `activation` with the inspected
   values and mirror pairs.
7. Change `status` to `signed`, then run:

   ```bash
   node --test scripts/validate-cr07-activation.test.mjs
   node scripts/validate-cr07-activation.mjs
   ```

8. Publish only after CI passes and independent security review and
   multi-operator rehearsal evidence are complete.

The gate validates repository artifacts. It cannot attest that an external URL
serves the same bytes, that a private-key ceremony was honest, or that operators
completed the rehearsal; those remain external deployment evidence.
