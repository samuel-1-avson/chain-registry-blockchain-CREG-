# CREG PyPI authority verifier

This package is the repository implementation of the PyPI half of CREG's
authority-adapter v2 boundary. It verifies exact, already-fetched bytes. It
never fetches PyPI, Sigstore, TUF, or transparency-log state itself.

The verifier accepts a single canonical v2 invocation on stdin and emits one
RFC 8785 JSON result, with no newline, only after all of these checks succeed:

- the exact Integrity API response and distribution bytes match the invocation
  digests;
- the URLs identify the same exact PyPI filename, project, and version;
- a hash-pinned policy authorizes the exact namespace, PyPI publisher object,
  OIDC issuer, and certificate subject;
- exactly one PEP 740 PyPI Publish Attestation binds that filename and SHA-256;
- Sigstore verifies its DSSE signature, certificate chain, transparency-log
  inclusion proof/checkpoint, and identity against the supplied TrustedRoot;
- exactly one `creg-claim.json` or `creg-recovery.json` occurs at the
  policy-defined archive path and is RFC 8785-equivalent to the signed
  challenge; and
- the verified transparency-log integration time, certificate, challenge, and
  local verification times overlap safely.

The stdin framing ceiling is 145 MiB. Rust's hard maxima are a 64 MiB artifact
and 8 MiB evidence object, and v2 transports both as lowercase hex:
`2 × (64 MiB + 8 MiB) = 144 MiB`. The remaining 1 MiB is a conservative budget
for the bounded invocation envelope, URLs, identity fields, challenge, JSON
syntax, and worst-case JCS string escaping. This is a protocol ceiling, not a
recommended operating size. The default 16 MiB artifact and 2 MiB evidence
limits are substantially smaller, and process memory limits must account for
the canonical JSON, decoded bytes, archive parsing, and Sigstore verification.

The archive is inspected without extracting to disk. ZIP central-directory
counts are bounded before per-entry parser objects are allocated, and GZIP/TAR
contents are decompressed and parsed through a bounded stream. Absolute,
traversing, duplicate, case-colliding, linked, special, sparse, encrypted,
overlapping, oversized, and excessive-ratio entries are rejected.

## Activation trust material is deliberately not checked in

A reviewed production verifier artifact must have these immutable sibling
files:

- `pypi-policy.json`
- `pypi-trusted-root.json`

`pypi-policy.json` must conform to `pypi-policy.schema.json` and must itself be
RFC 8785 canonical JSON. Each publisher entry pins the complete publisher
identity returned by PyPI, plus the exact OIDC issuer and certificate SAN that
Sigstore verifies. The activation manifest's `policy_sha256` must hash the
exact bytes.

`pypi-trusted-root.json` must be an independently reviewed Sigstore production
TrustedRoot obtained through the operator's signed TUF/root ceremony. The
activation manifest's `trust_root_sha256` must hash the exact bytes. The parent
`trust-candidates/` directory records an exact TUF-acquired
`candidate_unapproved` snapshot for reproducible review; the verifier does not
search or trust it, and the repository gate prevents that candidate from being
activated while unapproved. This repository does not bundle a runtime
"latest" downloader, placeholder root, or positive signature fixture.
Supplying and approving production trust material remains an operational
security ceremony.

Both files must be mounted read-only beside the installed entry point. The
verifier reads each through a bounded non-symlink file descriptor, hashes the
result, and parses the TrustedRoot directly from those same pinned in-memory
bytes. Validators should additionally run the process as a low-privilege user
with network denial, a read-only executable environment, and OS
CPU/memory/process limits.

## Locked development installation

The project uses exact direct-dependency pins and a committed `uv.lock` with
the complete transitive resolution. This installation is suitable for
development and tests:

```text
cd tools/authority-adapters/pypi-verifier
uv sync --frozen
uv run --frozen python -m unittest discover -s tests -v
```

The generated `creg-pypi-authority-verifier` console launcher is **not** an
acceptable production `verifier_sha256` artifact. Its bytes contain an import
stub; hashing it does not bind `sidecar.py`, the Python interpreter, or imported
packages. A virtual environment built from `uv.lock` improves reproducibility
but does not change that runtime integrity fact.

The repository now includes a locked PyInstaller 6.16.0 one-file recipe. It
pins the build interpreter through `.python-version`, disables UPX and bytecode
optimization, canonicalizes PyInstaller's otherwise order-dependent standard
library ZIP, performs two clean same-host builds, rejects unequal bytes, runs a
fail-closed process smoke test, and prints the final SHA-256:

```text
uv sync --frozen --python 3.11.14 --group build
uv run --frozen --group build python scripts/build_onefile.py --verify-reproducible
```

The ignored `dist/creg-pypi-authority-verifier[.exe]` output is a single
content-addressable file containing the verifier source, CPython runtime, and
resolved dependency graph. The hash is operating-system, architecture, and
CPython-build specific. Every target therefore needs its own independently
reviewed build and same-target reproduction; a Windows hash cannot approve a
Linux artifact.

PyInstaller one-file executables must unpack native libraries before Python
starts. The build smoke models the required node contract: start with a cleared
environment, create a new absolute private per-invocation scratch directory,
export only `CREG_AUTHORITY_ADAPTER_PROTOCOL=2` plus `TEMP`, `TMP`, and
`TMPDIR` pointing to that same directory, hold it through child exit, and
remove it afterward. The production implementation must create that directory
below an absolute node-data scratch root with mode `0700` on Unix or an
equivalent service-only ACL on Windows. It must not preserve ambient temp
variables or make the executable/policy/trust directory writable.

The Rust launcher now creates and removes a private per-invocation directory
below the canonical node data directory, clears the ambient environment, and
exports only `CREG_AUTHORITY_ADAPTER_PROTOCOL=2` plus `TEMP`, `TMP`, and
`TMPDIR` pointing to that directory. Local Windows node-to-frozen rejection
through that launcher is recorded; a freshly rebuilt one-file candidate still
has to match the reviewed hash and complete the target test matrix.
Production activation remains disabled until that evidence exists together
with an independently reviewed target artifact, production policy/root, live
positive fixture, signed activation spec, and operator rehearsal. Configure
`CREG_NAMESPACE_AUTHORITY_PYPI_VERIFIER_PATH` only after those gates. If
deployment instead needs a directory or container image, the Rust/config
contract must first be extended to verify that complete bundle before launch.
A self-check inside imported Python code cannot repair an unbound launcher
because modified code could remove the check.

An end-to-end positive production run intentionally remains gated on a real,
fresh package containing a valid CREG challenge, the exact PyPI provenance
object, an approved policy entry, and the independently pinned production
TrustedRoot. Unit tests cover rejection, binding, archive, and process
fail-closed behavior without inventing a trust root or signature.

To run that gated check, set `CREG_PYPI_PRODUCTION_FIXTURE_DIR` to an
independently reviewed directory containing the two sibling deployment files
and `authority-adapter-invocation-v2.jcs.json`, then rerun the frozen test
command. The invocation must still be inside its short challenge/certificate
window; an expired captured fixture is rejected.

## Security boundary

PEP 740 retains the package-index trust assumption: the provenance wrapper and
publisher grouping are served by PyPI. Sigstore authenticates the attestation
and workload identity; it does not prove that arbitrary unsigned registry JSON
is truthful. CREG therefore also pins the publisher policy, binds exact
artifact/evidence bytes, and requires independent validators to quorum-sign the
result before it becomes chain authority.

This implementation does not enable npm. npm remains fail-closed until npm
publishes an official cryptographically verifiable signal that distinguishes
Trusted Publishing from token-authenticated publication with generic
provenance.
