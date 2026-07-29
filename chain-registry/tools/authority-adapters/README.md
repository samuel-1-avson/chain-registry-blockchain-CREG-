# CREG namespace-authority adapter contract

Status as of 2026-07-29: the protocol v2 contract and rejection fixture remain,
`pypi-verifier/` provides a real offline PyPI PEP 740/Sigstore verifier, and
`npm-verifier/` provides real offline npm publish-attestation, provenance, and
tarball pre-verification. Both deliberately ship without activated production
policy. The repository contains a separately gated, independently unapproved
upstream trust-material candidate so acquisition bytes and hashes can be
reviewed without making them runtime authority. npm additionally terminates at
an unconditional authentication-mode gate because public npm evidence cannot
prove Trusted Publishing OIDC rather than token-authenticated publication.

The Rust boundary in `crates/node/src/authority_adapter.rs` is deliberately
fail-closed. A sidecar may emit `verified: true` only after it
cryptographically verifies upstream registry evidence and proves that the
fresh `creg-claim.json` or `creg-recovery.json` challenge is inside the exact
attested artifact.

Parsing registry JSON and copying its fields into a success response would turn
caller-controlled input into namespace authority and is prohibited.

## Artifact-input gap closed in protocol v2

`AuthorityAdapterRequestV2` gives the pinned sidecar all immutable package
inputs needed for verification without a second registry fetch:

- the exact evidence URL, bytes, and SHA-256;
- the exact artifact URL and bytes;
- the registry-native artifact digest (`sha512` for npm and `sha256` for
  PyPI); and
- the expected registry identity plus the fresh, controller-signed challenge.

The Rust caller limits both byte streams, decodes their lowercase hexadecimal
representations, recomputes their digests, and restricts artifact URLs to the
official registry origins. `AuthorityAdapterResultV2` must echo the artifact
URL, digest algorithm, and digest, and the caller checks those fields against
the request before accepting an observation.

This closes request v1's artifact-input gap. npm's verification design binds
signed provenance to a package PURL and tarball SHA-512, while PyPI's official
verification flow consumes both the distribution and its attestations:

- <https://github.com/npm/provenance#server-side-provenance-verification-and-proof>
- <https://docs.pypi.org/attestations/consuming-attestations/>
- <https://pypi.github.io/pypi-attestations/pypi_attestations.html>

The sidecar must not refetch mutable registry state. Its output remains an
out-of-consensus observation that independently verifying validators must turn
into a chain-bound quorum certificate.

`verified_at` remains a required sidecar-result field so each validator can
apply freshness and clock-skew checks to its own invocation. It is
**validator-local metadata and MUST NOT enter the shared
`NamespaceAuthorityProof`, `NamespaceAuthorityStatement`, or their
digest/signing identity**. Validators can therefore verify at different local
times while otherwise identical observations converge on one shared statement
and aggregate into the same quorum certificate.

Authority claim and recovery API endpoints are protected by the node's operator
ACL; public callers cannot invoke the external verifier boundary directly. The
ACL accepts only the secret `CREG_OPERATOR_API_KEY`; `CREG_OPERATOR_PUBKEY` is
public verification material and cannot authenticate a private route. The
runtime also enforces a hard maximum of **4 concurrent authority-verifier
invocations**, regardless of configured concurrency.

## Remaining production gates

### 1. npm authentication mode is not proven by provenance alone

The CREG authority enum is specifically `npm_trusted_publisher`. npm documents
both automatic provenance for trusted publishing and token-authenticated
`npm publish --provenance` using `NODE_AUTH_TOKEN`:
<https://docs.npmjs.com/generating-provenance-statements/>.

Public provenance proves a CI workload and artifact, but its presence alone
does not distinguish a trusted-publisher OIDC exchange from token-based
publication. CircleCI also demonstrates why automatic provenance is not a
reliable authentication-mode flag: npm supports CircleCI trusted publishing
but currently says automatic provenance is unavailable there:
<https://docs.npmjs.com/trusted-publishers/>.

A production npm adapter must remain unavailable until an official,
cryptographically verifiable registry signal distinguishes trusted publishing
from token-plus-provenance. It must not silently reclassify generic provenance
as `npm_trusted_publisher`.

### 2. Trust roots, policy, dependencies, and executable bytes must be immutable

The PyPI verifier uses the supplied hash-pinned Sigstore TrustedRoot directly
and never refreshes TUF or performs a network request. npm's documented path
similarly uses Sigstore plus npm registry keys.

The Rust configuration pins `policy_sha256` and `trust_root_sha256`. A
production rollout still needs:

1. an immutable offline Sigstore/TUF trust snapshot and, for npm, a separately
   approved registry-key snapshot shipped beside the binary;
2. a canonical policy defining issuers, identities, predicates, archive rules,
   and clock policy;
3. an independently reviewed self-contained verifier artifact whose hashed
   bytes cover the runtime, implementation, and dependency graph; and
4. startup and runtime checks that hash those exact deployment files and return
   their digests.

No test fixture, runtime latest-root network refresh, or caller-provided root
may stand in for those deployment artifacts.

`trust-candidates/` currently records a Sigstore production TrustedRoot
obtained through sigstore-python 4.5.0's TUF client and npm registry keys
obtained from npm's official HTTPS endpoint. The offline gate validates their
exact hashes, schema, key algorithms, retrieval-time validity, and unapproved
state. The candidate manifest SHA-256 is
`74833ae4bd2116325fe2f16c53985138186b2fa6e7e2afa953248499be70e985`;
the TrustedRoot SHA-256 is
`6494e21ea73fa7ee769f85f57d5a3e6a08725eae1e38c755fc3517c9e6bc0b66`;
and the npm key-response SHA-256 is
`faf23d8753d5bb79df250f10391ac89b63ecf7743e48487a544a99c847f9c8df`.
These are review candidates, not activation pins: independently controlled
operators must reproduce and approve them, and namespace-specific publisher
policies must still be supplied and signed into the rollout.

The generated Python `creg-pypi-authority-verifier` console launcher is a
development/test entry point, not a production verifier artifact: its hash
does not cover `sidecar.py`, the interpreter, or imported packages. The PyPI
directory now contains a locked PyInstaller one-file recipe that performs two
clean same-host builds, requires byte-identical outputs, smoke-tests a cleared
environment with one private absolute `TEMP`/`TMP`/`TMPDIR`, and prints the
artifact SHA-256. Its outputs are target-platform/architecture/CPython-build
specific and are ignored rather than treated as reviewed releases. The Rust
launcher now creates and removes one private per-invocation directory below
the canonical node data directory, clears the ambient environment, and passes
only the protocol and temporary-directory variables to the child. PyPI must
remain disabled until that boundary passes the complete target test matrix and
a target build plus real policy/root/positive fixture completes independent
review. A directory/image bundle would require a Rust/config extension that
verifies that whole bundle; an in-process Python self-check is not an integrity
boundary.

The npm verifier's `npm run build:sea` path instead bundles its lockfile-pinned
JavaScript dependency graph into the exact Node runtime used for the build,
smoke-tests the resulting Node SEA, and prints its SHA-256. Operators must pin
that runtime input, review the build, place approved policy/root/key files next
to the SEA, and put the final executable hash in the activation manifest.

### 3. Challenge placement and archive handling are implemented

The PyPI implementation enforces deterministic, policy-pinned in-memory archive
rules:

- exact permitted challenge paths for npm tarballs, Python wheels, and source
  distributions;
- rejection of duplicate challenge entries;
- rejection of absolute paths, `..`, symlinks, hard links, device entries,
  overlapping ZIP entries, and decompression bombs;
- maximum artifact, entry, and extracted-byte limits; and
- byte-for-byte or RFC 8785 comparison of the embedded challenge.

The npm verifier independently enforces a strict gzip/USTAR-only profile,
rejects extension and link entry types, validates headers and canonical archive
termination, applies entry/size/ratio limits, binds `package/package.json`, and
requires exactly one canonical challenge document.

### 4. Process isolation and archive operations remain deployment controls

The node verifies the configured file's hash, clears its environment, bounds
its input/output, and applies a timeout. The hash protects the verifier only
when that file is the complete reviewed artifact; it does not transitively bind
Python imports. These controls also do not create an OS sandbox or a
low-privilege execution identity. A production deployment must deny sidecar
network access, apply CPU/memory/process limits, mount the complete executable,
policy, and trust roots read-only, and prevent replacement between hash
verification and launch.

The content-addressed archive now enforces one shared byte/entry quota for
evidence and artifacts, a minimum-retention window, atomic mutation, and
pressure-only garbage collection of retention-eligible entries. Operators must
still size the three `CREG_NAMESPACE_AUTHORITY_ARCHIVE_*` settings, configure
capacity alerts, and rehearse quota exhaustion and recovery before accepting
public authority-attestation traffic.

## Registry-specific implementation requirements

### PyPI

The repository implementation uses the official `pypi-attestations` API:
<https://pypi.org/project/pypi-attestations/>.

It:

1. parse the exact Integrity API provenance object;
2. select a publisher bundle matching pinned identity policy;
3. verify the PyPI Publish Attestation with the pinned offline Sigstore trust
   snapshot;
4. match the exact distribution filename and SHA-256 to one attestation
   subject;
5. require predicate type
   `https://docs.pypi.org/attestations/publish/v1`;
6. verify the canonical package namespace; and
7. safely extract and compare exactly one fresh CREG challenge.

PyPI documents the Integrity API at
<https://docs.pypi.org/api/integrity/>. PEP 740 explicitly retains the
index-trust assumption:
<https://peps.python.org/pep-0740/#index-trust>.

### npm

The repository implementation follows npm's Sigstore semantics: it verifies
the registry publish attestation with a policy-pinned npm registry key,
verifies the provenance certificate and transparency material against a
hash-pinned TrustedRoot, and binds both attestations to the same exact PURL and
tarball SHA-512:
<https://github.com/npm/provenance#verifying-attestations-with-npm-audit-signatures>.

Those checks do not remove the authentication-mode gate above.

## Safe fixtures in this package

- `schemas/authority-adapter-request-v2.schema.json` mirrors the strict request
  shape.
- `schemas/authority-adapter-result-v2.schema.json` mirrors the strict result
  shape. Passing that schema never proves a signature or request/result
  equality.
- `src/rejection-stub.mjs` accepts protocol v2 framing but is intentionally
  incapable of producing stdout or a successful exit.
- `test/contract.test.mjs` checks the v2 artifact bindings and fail-closed
  process behavior.
- `pypi-verifier/` contains the offline cryptographic verifier, strict policy
  schema, hash-locked Python dependency graph, archive/adversarial tests, and a
  gated real-fixture test. It contains no activated policy instance or
  fabricated signature.
- `npm-verifier/` contains the offline cryptographic pre-verifier, strict
  policy schema, lockfile-pinned dependency graph, strict USTAR/adversarial
  tests, gated real-fixture test, and self-contained Node SEA build. It
  contains no activated policy or fabricated signature, and it cannot pass the
  authentication-mode gate.
- `trust-candidates/` contains exact unapproved upstream trust bytes plus a
  canonical manifest; its gate prevents a guarded chain spec from treating
  that candidate as approved authority.

Run the zero-dependency tests with:

```text
npm test --prefix tools/authority-adapters
```

Run the frozen PyPI verifier tests with:

```text
uv run --directory tools/authority-adapters/pypi-verifier --frozen \
  python -m unittest discover -s tests -v
```

Run the locked npm verifier checks and build its self-contained artifact with:

```text
npm ci --prefix tools/authority-adapters/npm-verifier --ignore-scripts
npm run check --prefix tools/authority-adapters/npm-verifier
npm audit --prefix tools/authority-adapters/npm-verifier
npm run build:sea --prefix tools/authority-adapters/npm-verifier
```

Never configure the rejection stub as a production verifier, and never add a
success fixture backed by invented signatures, placeholder trust roots, or
unchecked registry JSON.
