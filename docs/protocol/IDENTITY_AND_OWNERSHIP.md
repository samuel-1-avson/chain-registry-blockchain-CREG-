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

Until an ecosystem authority adapter is available, CREG can preserve only a
local controller relationship. Its namespace prefix is exactly
`<ecosystem>:<name>@`; the trailing marker prevents `npm:foo` from matching
`npm:foobar`. A local controller is not evidence of upstream registry
ownership and must never be presented as such.

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

### Signed local lifecycle events

`POST /v1/publisher/namespaces/lifecycle` accepts a versioned
`delegate` or `transfer` event only when both the persisted current controller
and the proposed successor sign the same domain-separated payload. The payload
binds the namespace, action, both controller keys, nonce, issuance time, and
expiry. The node rejects a conflicting current controller, duplicate durable
nonce, future-dated or expired event, and a lifecycle event that was not live
at the canonical block timestamp. The controller index is rebuilt only from
canonical blocks after a restart or reorg.

`recover` deliberately remains unavailable. It may omit the old controller's
signature, but it needs a verified npm/PyPI authority proof before it can
change a controller. Both the HTTP endpoint and block replay reject recovery
until the authority adapter described below exists. This prevents a peer or
block producer from bypassing the endpoint's fail-closed behavior.

The remaining production requirement is authoritative ownership proof from an
ecosystem adapter. Until an adapter and a fresh proof exist, the status is
`ownership_unconfirmed`; the current local controller must not be treated as
profile qualification.

## Authority-adapter contract (required before production use)

An adapter is the only component that may change an ownership proof from
`ownership_unconfirmed` to authoritative. It MUST:

1. Accept only `npm` or `pypi` proof records whose authority, namespace,
   audience, digest, and controller identities first pass the versioned schema
   checks.
2. Retrieve evidence only from an allowlisted, HTTPS or content-addressed
   origin. It MUST enforce a bounded response size, verify the SHA-256 digest,
   and reject redirects to a different authority domain.
3. Verify the OIDC/Sigstore/provenance signatures against a pinned issuer,
   allowed signing algorithms, cached JWKS/trust root, expected registry
   audience, and a workload subject policy. Discovery or key-fetch failure is
   `ownership_unconfirmed`, never a pass.
4. Verify that the registry evidence binds the same namespace and release
   artifact/source subject as the CREG submission. It must also bind the
   controller EVM and Ed25519 identities through the CREG v3 signature.
5. Record the adapter version, trust-root fingerprint, verification time, and
   evidence digest in the append-only ownership event. Never persist raw OIDC
   bearer tokens or unredacted private claims.

The initial allowlist should use the registry audience `npm` for npm trusted
publishers and `pypi` for PyPI trusted publishers. npm provenance and PyPI
attestations are evidence inputs; neither is a statement that the software is
safe.

## Required evidence

Ownership evidence records ecosystem, namespace, authority source, lookup
epoch, delegation chain, controller identity, expiry, and signature. An
upstream outage yields `ownership_unconfirmed`, not approval.
