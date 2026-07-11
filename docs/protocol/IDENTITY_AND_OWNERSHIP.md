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

## Required evidence

Ownership evidence records ecosystem, namespace, authority source, lookup
epoch, delegation chain, controller identity, expiry, and signature. An
upstream outage yields `ownership_unconfirmed`, not approval.
