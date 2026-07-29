# CR-07 namespace-authority activation specification

> **Status:** Normative rollout specification; no production activation is
> authorized by this document.
>
> **Current repository state:** `disabled`. The checked-in manifest contains no
> activation height, production trust root, production policy, production
> verifier digest, or production signature.

## 1. Purpose

CR-07 changes first-claim and recovery authorization for npm and PyPI
namespaces. After activation, a local first submitter cannot become the
authoritative controller merely by submitting first. The claim or recovery
must instead carry a fresh upstream-authority observation certified by the
validator set.

This specification defines the only permitted transition from the
pre-activation compatibility state to that consensus rule. It does not create
an authority, select a signing key, or assert that any public deployment has
completed the transition.

## 2. Security invariants

An activation is valid only when all of the following are true:

1. The activation height and authority-policy pins are fields of an exact,
   signature-verified chain spec. Environment variables cannot add or replace
   them.
2. The chain spec binds the chain ID, computed genesis hash, activation height,
   adapter identity/version, and SHA-256 digests of the complete verifier
   artifact, policy, and trust-root bundle.
3. The verifier artifact is self-contained for the execution model that the
   node hashes and launches. Hashing a Python or JavaScript launcher while
   loading unpinned adjacent modules is not sufficient.
4. A self-extracting verifier receives a fresh, absolute, validator-private
   scratch directory for each invocation. The node clears the child
   environment, exports only the protocol variable and that directory through
   `TEMP`, `TMP`, and `TMPDIR`, holds it until child exit, and removes it after
   success, rejection, timeout, or spawn/I/O failure. Ambient temp inheritance
   is forbidden.
5. Every attesting validator fetches the official evidence and artifact bytes
   independently, applies byte and concurrency limits, verifies the pinned
   policy and trust root, and archives the accepted inputs.
6. npm authority remains fail closed unless an official, cryptographically
   bound registry signal distinguishes trusted-publisher authentication from
   token-authenticated publication with provenance. Provenance alone is not
   that signal.
7. PyPI authority accepts only a valid PEP 740 publication attestation and
   Integrity API response whose distribution identity, artifact digest,
   trusted-publisher identity, embedded CREG challenge, and pinned policy all
   agree.
8. A claim or recovery becomes consensus authority only after distinct
   signatures from `floor(2n/3) + 1` members of the authenticated prior
   validator set. One sidecar result is never consensus authority.
9. A real activation uses a new, independently controlled signing key. Keys
   disclosed in repository fixtures or development tooling are disallowed.
10. Mandatory mode is not enabled until independently controlled operators
   complete the rehearsals and evidence review in this specification.

## 3. Signed artifacts

### 3.1 Chain spec

The signed chain spec MUST use `spec_version: 1` and the signature domain
`creg-chain-spec-v1`. Its `consensus_params` MUST contain:

- one non-zero future `namespace_authority_activation_height`; and
- `namespace_authority_policies` containing at least one exact
  `npm_trusted_publisher` policy and one exact `pypi_trusted_publisher` policy.

Each policy has exactly:

| Field | Requirement |
|---|---|
| `authority` | `npm_trusted_publisher` or `pypi_trusted_publisher` |
| `adapter_id` | Canonical reviewed adapter identity |
| `adapter_version` | Immutable reviewed release version |
| `verifier_sha256` | SHA-256 of the complete executable artifact launched by the node |
| `policy_sha256` | SHA-256 of the canonical policy bytes interpreted by that artifact |
| `trust_root_sha256` | SHA-256 of the complete offline trust-root bundle interpreted by that artifact |

The detached Ed25519 signature covers
`creg-chain-spec-v1|<RFC8785-canonical-full-spec>`.

### 3.2 Activation manifest

`chain-registry/config/cr07-activation-manifest.json` is the
repository-distribution gate. In `signed` state its `activation` object MUST
bind:

- primary spec and signature paths;
- every repository mirror and its signature path;
- chain ID and non-placeholder computed genesis hash;
- non-disallowed chain-spec signing public key;
- non-zero activation height;
- SHA-256 of the RFC 8785-canonical full spec; and
- SHA-256 of the canonical `creg-cr07-activation-v1` payload containing the
  chain ID, genesis hash, activation height, and ordered policy objects.

Every selected mirror MUST be canonically identical to the primary spec and
carry the identical detached signature. Every guarded but unselected spec MUST
remain unactivated.

The manifest signature is not a substitute for chain-spec verification. The
gate verifies the chain-spec signature itself.

### 3.3 Verifier release record

Each verifier release MUST publish and retain:

- source revision and review commit;
- reproducible build instructions and pinned toolchain/dependencies;
- final executable SHA-256;
- policy and trust-root SHA-256 values;
- SBOM and build provenance bound to the final executable;
- supported operating systems and sandbox profile;
- maximum canonical request size, peak-memory budget, scratch-directory
  lifecycle, and a cleared-environment frozen-artifact invocation result;
- complete conformance and adversarial-test results; and
- independent reviewer identity, decision, and review date.

If the build cannot produce one self-contained executable, CR-07 requires a
future protocol revision that defines and verifies a deterministic bundle or
container-image digest before execution. Operators MUST NOT reinterpret the
current single-file pin as a directory pin.

## 4. Policy and trust requirements

Policies and trust bundles are immutable inputs to one activation. Silent
network discovery, trust-on-first-use, mutable `latest` references, ambient
credentials, and online root replacement are forbidden.

The npm policy MUST:

- verify the signed provenance/attestation envelope and exact tarball
  SHA-512;
- bind package, version, canonical npm namespace, official artifact URL,
  publisher/workflow identity, and the embedded signed CREG intent;
- enforce issuer, audience, subject/repository/workflow/environment rules; and
- reject until an official signed authentication-mode claim proves trusted
  publishing rather than token publication.

The PyPI policy MUST:

- verify the Integrity API response and each relevant PEP 740 DSSE envelope
  offline against the pinned Sigstore trust root;
- bind normalized project name, version, distribution filename, exact
  artifact SHA-256, publisher identity, and the embedded signed CREG intent;
- enforce issuer, audience, subject/repository/workflow/environment rules; and
- reject missing, ambiguous, expired, revoked, or extra conflicting
  attestations.

Trust-root rotation is a new signed-spec rollout. It MUST NOT mutate the bytes
behind an existing digest.

## 5. Ceremony and rollout

The release coordinator MUST produce a retained evidence bundle for these
ordered gates:

1. Independent review approves the exact verifier, policy, trust, SBOM, and
   provenance artifacts.
2. At least two independently controlled operators reproduce the artifact
   digests and policy decisions.
3. Operators run positive PyPI fixtures and all negative npm/PyPI fixtures
   through the exact frozen executable launched by the node, including the
   cleared environment and private scratch-directory contract.
   npm positive activation is prohibited while the required official
   authentication-mode signal is unavailable.
4. Operators rehearse verifier timeout, crash, oversized input, malformed
   archive, quota exhaustion, trust-root mismatch, policy mismatch, artifact
   substitution, challenge replay, stale challenge, and conflicting
   controller cases.
5. A future activation height is selected with sufficient upgrade notice.
6. An offline, independently witnessed key ceremony signs the exact candidate
   chain spec. Private keys never enter the repository, logs, or evidence
   archive.
7. The signed spec and detached signature are copied byte-for-byte to every
   declared mirror. The activation manifest is changed to `signed`.
8. The repository activation validator, full Rust/contract suites, clean
   deployment migration, and operator canary all pass.
9. Independently controlled validators confirm the same signed spec and remain
   healthy through the activation height.

The rollout record MUST identify each operator's control domain. Multiple
processes, hosts, or cloud accounts controlled by one person or organization
do not constitute independent operators.

## 6. Failure and recovery

Before the activation height, a failed gate aborts the rollout and the guarded
specs remain unactivated. Do not move activation fields into environment
variables to bypass the gate.

After activation:

- missing verifier, trust, policy, archive capacity, upstream evidence, or
  npm authentication-mode proof yields `ownership_unconfirmed`;
- verifier disagreement does not select a majority sidecar response—only a
  valid validator-set quorum certificate can enter consensus;
- a reorg invalidates certificates whose anchors are no longer canonical;
- a compromised verifier/policy/trust root requires an incident record and a
  new signed-spec transition; and
- disabling the rule or moving the activation height backward requires an
  explicitly specified consensus migration and independent review. It is not
  an operator environment toggle.

## 7. Acceptance evidence

Production activation is not complete until the remediation tracker links to:

- exact signed spec, detached signature, mirror hashes, and manifest result;
- verifier/policy/trust hashes and retained release records;
- independent cryptographic review;
- full clean test/CI results;
- multi-operator activation, replay, partition, reorg, rebuild, long-range,
  process-kill, archive-pressure, and rollback rehearsals; and
- the post-activation canonical block and validator-set observations.

Repository fixtures, synthetic keys, disabled manifests, schemas, or local test
passes are useful controls but do not satisfy these external evidence gates.
