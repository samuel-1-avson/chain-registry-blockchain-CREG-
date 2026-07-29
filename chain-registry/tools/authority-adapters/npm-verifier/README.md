# CREG npm authority verifier

This package implements the cryptographic pre-verification half of CREG's npm
authority-adapter v2 boundary. It consumes only exact bytes already supplied
by the Rust caller and performs no network requests.

It intentionally cannot return a successful `npm_trusted_publisher` result.
After every available cryptographic and archive check succeeds, it exits
non-zero with
`ERR_NPM_TRUSTED_PUBLISHING_AUTH_MODE_UNPROVEN`.

## Why the final gate is mandatory

npm's public evidence does not currently contain a signed authentication-mode
field.

- npm documents that Trusted Publishing uses OIDC and that the CLI falls back
  to traditional tokens:
  <https://docs.npmjs.com/trusted-publishers/#how-trusted-publishing-works>.
- npm automatically creates provenance for GitHub/GitLab Trusted Publishing,
  but separately documents token-authenticated
  `npm publish --provenance` with `NODE_AUTH_TOKEN`:
  <https://docs.npmjs.com/generating-provenance-statements/>.
- npm's signed publish predicate contains only package name, version, and
  registry:
  <https://github.com/npm/attestation/tree/main/specs/publish/v0.1>.
- npm's verifier treats the publish attestation as proof that an authorized
  user or token was accepted; it does not classify the authentication method:
  <https://github.com/npm/provenance#verifying-attestations-with-npm-audit-signatures>.
- CircleCI Trusted Publishing currently has no automatic provenance, so the
  presence or absence of provenance is not an authentication-mode bit:
  <https://docs.npmjs.com/trusted-publishers/#automatic-provenance-generation>.

Consequently, provenance proves the CI workload and artifact, while the
registry publish attestation proves registry acceptance. Neither proves that
the acceptance credential came from Trusted Publishing rather than a
traditional token. Treating generic provenance as `npm_trusted_publisher`
would silently broaden the authority model.

The policy schema fixes `authentication_mode.status` to `unavailable` and
`official_cryptographic_signal` to `null`. Changing a policy file cannot bypass
the code gate. Enabling npm requires a new reviewed adapter version after npm
defines and deploys an official signed field, plus a policy schema that pins
that field and its exact semantics.

## Checks completed before the gate

The verifier:

- validates the canonical v2 request shape and controller-signed CREG intent;
- hashes the exact evidence bytes with SHA-256 and the exact tarball with npm's
  native SHA-512;
- binds the evidence URL, tarball URL, `package/package.json`, package name,
  exact version, npm PURL, challenge namespace, and both signed attestation
  subjects;
- requires exactly one official npm publish attestation and one
  policy-selected SLSA provenance attestation;
- verifies the registry-key DSSE publish signature, the keyless provenance
  signature and exact certificate issuer/SAN, certificate chain and SCT, Rekor
  body, inclusion proof, checkpoint, and integrated times using Sigstore's
  verifier;
- uses only the invocation-hash-pinned TrustedRoot, the
  invocation-hash-pinned canonical policy, and registry keys whose exact file
  hash is pinned inside that policy; and
- scans the gzip/USTAR bytes in memory for exactly one CREG document at
  `package/creg-claim.json` or `package/creg-recovery.json`, comparing its
  RFC 8785 representation with the request.

The archive profile rejects non-USTAR extensions, links, special or sparse
entries, absolute or traversing paths, non-UTF-8/NFC paths, duplicates,
case-collisions, invalid checksums, malformed numeric fields, trailing
archives, oversized entries/totals/counts, excessive compression, and
challenge ambiguity. Nothing is extracted to disk.

The Sigstore bundle is decoded with `@sigstore/bundle` and verified through
`@sigstore/verify` with an explicitly supplied TrustedRoot and registry key
selector. This follows npm CLI's public-key handling for publish attestations
and certificate handling for provenance:
<https://github.com/npm/pacote/blob/main/lib/registry.js>.
Sigstore bundles carry the material needed for offline verification:
<https://docs.sigstore.dev/about/bundle/>.

## Activation trust material is deliberately absent

An installed verifier must have these immutable files next to the exact
executable selected by the activation manifest:

- `npm-policy.json`
- `npm-trusted-root.json`
- `npm-registry-keys.json`

No activated policy, signature, or positive cryptographic fixture is checked
in. The parent `trust-candidates/` directory records exact upstream
TrustedRoot and npm key-response candidates with an explicit
`candidate_unapproved` gate; those files are not searched or trusted by this
verifier. Operators must independently reproduce and approve the Sigstore
production TrustedRoot through their signed TUF/root ceremony and the npm
registry keys through a separately approved acquisition ceremony. The
activation manifest pins the exact policy and TrustedRoot bytes; the canonical
policy pins the exact registry-key file bytes.

All three files should be read-only and network acquisition must stay outside
the verifier process.

## Self-contained executable

`npm run build:sea` creates a Node single executable application at
`dist/creg-npm-authority-verifier` (or `.exe` on Windows). The build:

- bundles the verifier and every JavaScript dependency, rejecting any
  non-builtin external import;
- injects that bundle into the exact Node runtime used for the build;
- disables runtime command-line option injection through
  `execArgvExtension`;
- runs the built executable and requires its fail-closed smoke response to
  have non-zero status and empty stdout; and
- prints the SHA-256 of the final executable.

This SEA is the production-shaped, self-contained verifier artifact. Hash its
exact final bytes in the activation manifest. If platform signing is applied
after the build, calculate the activation hash after signing. Pin and record
the Node runtime input as part of the build provenance. Node documents the SEA
format and injection process at
<https://nodejs.org/api/single-executable-applications.html>.

`bin/creg-npm-authority-verifier.mjs` is only a source-development fallback.
Its absolute `/usr/bin/node` shebang prevents `PATH` substitution, but hashing
that small launcher does not cover the interpreter, sources, or imported
packages and is therefore not an acceptable production activation target.

## Locked installation and checks

Direct dependencies are exact pins and `package-lock.json` fixes the complete
transitive graph:

```text
cd tools/authority-adapters/npm-verifier
npm ci --ignore-scripts
npm test
npm run check
npm audit
npm run pack:check
npm run build:sea
```

`node_modules/`, coverage, package tarballs, `build/`, and `dist/` are ignored.

An optional end-to-end test can consume a fresh, independently reviewed
production fixture without adding it to this repository. Set
`CREG_NPM_PRODUCTION_FIXTURE_DIR` to a directory containing the three sibling
deployment files and `authority-adapter-invocation-v2.jcs.json`. The test
requires all real cryptographic checks to finish and then asserts that the
authentication-mode gate still rejects. Because the CREG challenge and
Sigstore certificate are deliberately short-lived, captured fixtures expire
quickly.
