# Identity and namespace ownership

## Identity classes

- Maintainer/CI: short-lived OIDC/Sigstore-compatible identity where available.
- Legacy publisher: EVM stake address plus Ed25519 signing key, bound by the
  dual-domain v2 payload.
- Validator/reviewer: registered EVM and Ed25519 identity with control-domain
  and infrastructure-domain disclosure.
- Witness: independent log operator with a separate signing key and retention
  domain.

## Namespace lifecycle

`unclaimed → claimed → delegated → transferred → rotated → recovered → disputed`.

The first CREG submission never overrides an authoritative upstream namespace.
Transfers require old/new controller authorization or an adjudicated recovery
event. A fork uses a distinct identity. A key rotation preserves the prior
history and names the rotation nonce.

## Local compatibility control

Before the signed CR-07 activation height, CREG preserves only a local
controller relationship. Its namespace prefix is exactly
`<ecosystem>:<name>@`; the trailing marker prevents `npm:foo` from matching
`npm:foobar`. A local controller is not evidence of upstream registry
ownership and must never be presented as such. After activation, legacy
publisher, rotation, and lifecycle transactions are rejected rather than
silently downgraded.

The CLI also refuses legacy publisher signing and legacy `creg multisig` by
default when no complete v4 identity is configured. Only a non-production,
pre-activation lab may opt in with
`CREG_ALLOW_LEGACY_PUBLISHER_PROOF=true`; `CREG_PRODUCTION=true` always
disables that escape hatch.

Both keys in a local key rotation sign the versioned payload below. The
namespace and nonce are signed so a rotation cannot be replayed for another
namespace owned by the same key:

```text
creg-publisher-key-rotation-v1
service:creg-package-ownership
namespace:<ecosystem>:<name>@
old_ed25519_pubkey:<old key, lowercase hex>
new_ed25519_pubkey:<new key, lowercase hex>
nonce:<monotonic integer>
```

### Signed lifecycle events

`POST /v1/publisher/namespaces/lifecycle` accepts a versioned
event whose domain includes the chain ID, genesis hash, canonical namespace,
action, current and next Ed25519/EVM controllers, durable nonce, issuance
time, expiry, and prior-event hash.

- `delegate` grants only the signed, expiring scope (initially `publish`) and
  retains the root controller.
- `transfer` requires the current root and successor to authorize the same
  event and changes the root controller.
- `recover` may omit the unavailable old-controller signature, but must carry
  a fresh npm/PyPI authority certificate for the recovery purpose and proposed
  controller.

The node rejects a conflicting controller, duplicate or stale durable nonce,
future-dated or expired event, cross-chain/genesis replay, and an event that
was not live at the canonical block timestamp. Controller, delegation, and
nonce indexes are rebuilt only from canonical blocks after restart or reorg.
Recovery remains fail-closed when no valid authority certificate is present.

Until real npm/PyPI verifiers, signed policy pins, and a validator quorum are
deployed, the status remains `ownership_unconfirmed`; a local controller must
not be treated as upstream authority or profile qualification.

## Authority-adapter contract (required before production use)

Authority verification is deliberately outside consensus. Every attesting
validator invokes its own pinned verifier, while block validation and rebuild
consume only the resulting quorum certificate and never make a live registry
request.

The workflow has two stages so it does not require an impossible artifact
self-hash:

1. The proposed controller creates and signs a fresh intent binding the chain,
   genesis, purpose (`claim` or `recover`), canonical namespace, current and
   proposed controllers, nonce, issuance time, and expiry. The intent does not
   contain the digest of the artifact that will contain it.
2. The publisher includes that exact intent in a new npm/PyPI release.
   Registry provenance or attestations identify the intent-containing artifact
   and its upstream publishing identity.
3. The request supplies both the official evidence URL and `artifact_url`.
   Each validator separately fetches bounded bytes from the authority-specific
   allowlisted origins, computes SHA-256 over the evidence, and computes the
   registry-native artifact identity: SHA-512 for npm tarballs and SHA-256 for
   PyPI distributions. Its pinned verifier must prove that the evidence,
   artifact digest, embedded intent, namespace, and upstream identity agree.
4. A certificate aggregates distinct signatures from
   `floor(2n/3) + 1` validators in the authenticated anchor set. The final
   publisher authorization or recovery event signs the completed certificate,
   so the certificate cannot be substituted after signing.
5. Consensus verifies certificate quorum, policy pins, chain/genesis,
   namespace/controllers, validity interval, and canonical block/set anchor.
   It never trusts one adapter response or fetches upstream data during replay.

The planned authenticated validator endpoints are:

- `POST /v1/validator/namespace-authority/attest` for one validator's
  independently verified statement and signature. Its request includes
  `artifact_url` as well as `evidence_url`;
- `POST /v1/validator/namespace-authority/certificates` for validation and
  digesting of a caller-assembled certificate containing matching validator
  statements.

Both routes require the validator operator's `CREG_OPERATOR_API_KEY`, supplied
as `X-Operator-Key` or `Authorization: Bearer`. This protects the local service
in addition to—not instead of—the controller-signed intent, validator-set
membership, individual attestation signature, and certificate quorum. The
attestation route has a dedicated limit of 2 requests per source IP per minute.
Verifier concurrency defaults to 2 and cannot be configured above 4.
`CREG_OPERATOR_API_KEY` is server-side operator material: never embed it in
`VITE_*`, browser JavaScript, public proxy configuration, URLs, or request
bodies.
`CREG_OPERATOR_PUBKEY` is public verification material and is not accepted by
the private API ACL.

These routes are reserved rollout interfaces and must be treated as
unavailable until they appear in the shipped OpenAPI document and pass the
recorded adversarial suite. Their presence alone does not prove that real
npm/PyPI verifier binaries are installed.

An adapter MUST:

1. Accept only `npm` or `pypi` proof records whose authority, namespace,
   audience, artifact identity, intent signature, digest, and controller
   identities first pass versioned schema checks.
2. Retrieve evidence and official artifact bytes separately from allowlisted
   HTTPS origins, with redirects disabled. It MUST enforce independent byte
   limits, compute the evidence SHA-256, compute native artifact SHA-512 for
   npm or SHA-256 for PyPI, and reject an origin, algorithm, or digest mismatch.
3. Verify the OIDC/Sigstore/provenance signatures against a pinned issuer,
   allowed signing algorithms, cached JWKS/trust root, expected registry
   audience, and a workload subject policy. Discovery or key-fetch failure is
   `ownership_unconfirmed`, never a pass.
4. Verify that registry evidence binds the same namespace and exact native
   artifact digest as the separately fetched `artifact_url`, and that the
   artifact contains the controller-signed intent. For npm, provenance alone
   is not sufficient: the policy must also require an official registry signal
   that the release used trusted-publisher authentication mode. A generic
   provenance envelope or an operator assertion cannot supply that signal.
   The final CREG v4 authorization separately binds the proof/certificate and
   package bytes.
5. Record the adapter/verifier version, executable hash, policy hash,
   trust-root hash, canonical anchor, exact evidence digest, native artifact
   digest, proof validity interval, and the verified npm authentication-mode
   signal where applicable. Record local completion time only in the local
   verifier audit trail. Exact evidence remains referenced as
   `creg-evidence:sha256:<digest>`. Artifact bytes are archived under
   `CREG_DATA_DIR/authority-artifacts/<sha256|sha512>/<prefix>/<digest>.bin`;
   the algorithm/digest pair is their content address. Never persist bearer
   tokens.

The sidecar may return `verified_at` so the invoking validator can reject a
stale or future local result. That completion time is not copied into the
shared `NamespaceAuthorityProof` or
`NamespaceAuthorityStatement`. The challenge, proof validity interval,
evidence/artifact digests, policy pins, and anchor determine the shared bytes,
allowing validators that finish at different seconds to sign the same
statement.

The registry audiences are `npm:registry.npmjs.org` for npm trusted publishing
and `pypi` for PyPI. npm provenance and PyPI attestations are evidence inputs;
neither is a statement that the software is safe.

### Signed-spec-only activation

`namespace_authority_activation_height` and
`namespace_authority_policies` are consensus parameters in the signed chain
spec. Every policy pins authority, adapter ID/version, verifier executable,
policy, and trust-root SHA-256 digests. Environment variables cannot safely
activate or replace these values; a node must fail startup if activation did
not come from a signature-verified spec.

Validator-local environment variables only locate and bound the pinned
processes:

| Variable | Purpose |
|----------|---------|
| `CREG_NAMESPACE_AUTHORITY_NPM_VERIFIER_PATH` | Absolute npm verifier executable path |
| `CREG_NAMESPACE_AUTHORITY_PYPI_VERIFIER_PATH` | Absolute PyPI verifier executable path |
| `CREG_NAMESPACE_AUTHORITY_VERIFIER_TIMEOUT_SECS` | Per-process deadline |
| `CREG_NAMESPACE_AUTHORITY_MAX_EVIDENCE_BYTES` | Exact input evidence limit |
| `CREG_NAMESPACE_AUTHORITY_MAX_ARTIFACT_BYTES` | Official artifact-byte limit; default 16 MiB, hard maximum 64 MiB |
| `CREG_NAMESPACE_AUTHORITY_MAX_RESULT_BYTES` | Canonical verifier-output limit |
| `CREG_NAMESPACE_AUTHORITY_MAX_CONCURRENT_VERIFICATIONS` | Per-node concurrency bound; default 2, hard maximum 4 |
| `CREG_OPERATOR_API_KEY` | High-entropy service ACL required by both authority POST routes |
| `CREG_ALLOW_LEGACY_PUBLISHER_PROOF` | Explicit non-production, pre-activation CLI compatibility; default false and ineffective when `CREG_PRODUCTION=true` |

## Required evidence

Ownership evidence records ecosystem, namespace, authority source, lookup
epoch, official `artifact_url`, native artifact identity, controller-signed
intent, controller identities, expiry, verifier/policy/trust-root pins,
canonical anchor, and validator signatures. npm evidence also records the
official trusted-publisher authentication-mode signal used by policy. An
upstream outage, missing signal, missing verifier, timeout, reorged anchor, or
pin mismatch yields `ownership_unconfirmed`, not approval.

Real npm/PyPI sidecars, official trust material, published policy bundles,
reviewed verifier artifacts, a signed activation spec, external operator
rehearsal, and independent security review remain deployment gates. Repository
schemas and adapter process boundaries are not substitutes for that evidence.
