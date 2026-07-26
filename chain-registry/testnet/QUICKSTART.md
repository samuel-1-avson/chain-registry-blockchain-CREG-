# Chain Registry — Quick Start Guide

> Goal: get a node running quickly and verify health/status surfaces.

**Current topology:** coordinated research alpha; the scale architecture is future design, not a deployment claim.
**External validators:** [VALIDATOR_ONBOARDING_CHECKLIST.md](../../docs/VALIDATOR_ONBOARDING_CHECKLIST.md)  
**Publishers/developers:** [PUBLIC_TESTNET_QUICKSTART.md](../../docs/PUBLIC_TESTNET_QUICKSTART.md)

## Choose your path

| Path | Time | What you get | Best for |
|------|------|--------------|----------|
| A. Single node (native) | 15 min | One node on your machine | Local development |
| B. Bootstrap host (Docker) | 15–20 min | Shared services + `node-1` | Local bootstrap |
| C. External validator | Not currently open | Future onboarding design | Authorized future cohort |


## Path A: Single node — native

```bash
git clone https://github.com/samuel-1-avson/chain-registry-blockchain-CREG-.git
cd chain-registry/testnet
cp .env.example .env
```

Linux/macOS: `./start-local-node.sh -n my-node`  
Windows: `.\start-local-node.ps1 -NodeId my-node`

```bash
curl http://localhost:8080/v1/health
curl http://localhost:8080/v1/chain/stats | jq
```

## Path B: Bootstrap host — Docker

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass -File testnet.ps1
```

Or:

```bash
docker compose --env-file .env.testnet -f docker-compose.testnet.yml up -d --build
```

## Path C: External validator (public testnet)

Follow the full checklist — do **not** use the retired 3-node Sepolia lab.

1. [VALIDATOR_ONBOARDING_CHECKLIST.md](../../docs/VALIDATOR_ONBOARDING_CHECKLIST.md)
2. [GENESIS_ALPHA_COHORT_PLAYBOOK.md](../../docs/GENESIS_ALPHA_COHORT_PLAYBOOK.md) (if invited)
3. Deploy with `docker-compose.validator.yml` or release `creg-node`
4. Point at `https://api.testnet.cregnet.dev` for registration smoke tests

## Supported smoke checks

```bash
curl http://localhost:8080/v1/health
curl http://localhost:8080/v1/chain/stats | jq '{height: .block_height, peers: .peer_count, validators: .validator_count}'
creg testnet status --node-url http://localhost:8080
```

## Publishing a package

```bash
creg --node-url http://localhost:8080 publish ./my-package.tgz \
  --key-file ~/.creg/publisher.key \
  --publisher-address 0xYourPublisherAddress
```

## Certified validator authority (staged rollout)

Leave `CREG_REQUIRE_CERTIFIED_COMMITTEES=false` until operators publish a real signed checkpoint certificate and its independent verification evidence. For a new or rebuilt node, place that exact single-line certificate JSON in `CREG_WEAK_SUBJECTIVITY_CHECKPOINT`; never select a checkpoint from gossip or one HTTP peer.

Start with an observer canary. It must retrieve a certificate history containing the exact configured checkpoint, verify each prior-set quorum and canonical block anchor, and reject missing/conflicting checkpoint history. Compare `GET /v1/validator/set-transitions` with two independently controlled peers before enabling mandatory mode. Follow the staged rollout and recovery procedure in [OPERATOR.md](./OPERATOR.md).

## Authoritative npm/PyPI ownership (CR-07)

CR-07 activation is not a local environment toggle. Its future activation
height and accepted npm/PyPI verifier, policy, and trust-root hashes must be
consensus parameters in a signature-verified chain spec. Leave
`CREG_NAMESPACE_AUTHORITY_NPM_VERIFIER_PATH` and
`CREG_NAMESPACE_AUTHORITY_PYPI_VERIFIER_PATH` empty unless the reviewed
binaries are mounted at those exact absolute paths. Empty or mismatched
verifiers must fail closed.

The two-stage flow avoids self-referential artifact hashing: a controller first
signs a fresh chain/genesis/namespace/controller intent, then publishes an
npm/PyPI artifact containing it. The request supplies both `evidence_url` and
`artifact_url`. Independently operated validators fetch the bounded bytes
separately, verify native artifact SHA-512 for npm or SHA-256 for PyPI, archive
the evidence and artifact content-addressably, and sign matching statements.
A `floor(2n/3) + 1` certificate is attached to the final package authorization
or recovery event; consensus never fetches upstream evidence.

The default artifact cap is 16 MiB
(`CREG_NAMESPACE_AUTHORITY_MAX_ARTIFACT_BYTES=16777216`) and the protocol hard
maximum is 64 MiB (`67108864`). npm provenance alone does not prove that npm
trusted-publisher authentication mode was used; a reviewed external
verifier/policy must require the official npm authentication-mode signal.

`POST /v1/validator/namespace-authority/attest` and
`POST /v1/validator/namespace-authority/certificates` are planned authenticated
interfaces, not currently guaranteed public endpoints. Both require
`CREG_OPERATOR_API_KEY` through `X-Operator-Key` or
`Authorization: Bearer`; this service ACL is additional to controller and
validator/quorum signatures. Attestation is limited to 2 requests per IP per
minute, with verifier concurrency default 2 and hard maximum 4. Sidecar
`verified_at` is local freshness data and is deliberately excluded from the
shared statement so independent validators sign identical bytes. Treat the
shipped OpenAPI document as authoritative. Real npm/PyPI sidecars, official trust
material, reviewed policy/verifier artifacts, a signed activation spec,
multi-operator rehearsal, and independent security review are still required
before testnet activation. See [OPERATOR.md](./OPERATOR.md) for rollout and
rollback.

Keep the operator key server-side. Do not add it to a `VITE_*` variable,
browser bundle, public nginx configuration, URL, or request body.
An operator public key is not a secret: `CREG_OPERATOR_PUBKEY` cannot
authenticate either private authority route.

Legacy publisher signing and `creg multisig` are disabled by default. Only an
intentional non-production, pre-activation lab may set
`CREG_ALLOW_LEGACY_PUBLISHER_PROOF=true`; `CREG_PRODUCTION=true` prevents the
escape hatch. Public testnet publishers must configure the v4 chain ID,
genesis, staking address, and controller identity instead.

## Next steps

1. Developers: Path A or B, then [PUBLIC_TESTNET_QUICKSTART.md](../../docs/PUBLIC_TESTNET_QUICKSTART.md)
2. Validators: Path C when cohort invites open
3. Maintainers: [OPERATOR.md](./OPERATOR.md) and [gcp/README.md](./gcp/README.md)
