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

On a clean operator machine, install the signed release first instead of
building Rust locally:

```bash
./scripts/install-creg.sh --version <release-tag>
```

```powershell
.\scripts\install-creg.ps1 -Version <release-tag>
```

Both installers require the release `SHA256SUMS` entry to match before
extracting an archive. Verify `SHA256SUMS.sig` and the GitHub provenance
attestation against the release policy before handing the binaries to an
operator. Once `creg` and `creg-node` are on `PATH`, the local starter prefers
those packaged binaries automatically; use `-NodeBinary`/`--node-binary` to
select an explicit path.

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

### Public services can start first

The GCP edge is not a validator. It may be bootstrapped before any external
validator or observer is online so operators can fetch the signed spec and use
the public services:

```powershell
.\testnet\gcp\deploy-stack.ps1 -BootstrapOnly
```

This starts the spec server, IPFS gateway, static explorer, faucet, and TLS
edge. The explorer UI is available immediately; CREG chain API reads return an
explicit `503 observer_unavailable` until the observer pool is attached. The
faucet still needs an approved Sepolia RPC, funded faucet account, and secret
injection. Once independent validators publish their peer/API endpoints,
provision the observer pool and run the normal edge deployment. Two independent
validators are needed for consensus and adversarial-rehearsal evidence, not for
this public-service bootstrap.

1. [VALIDATOR_ONBOARDING_CHECKLIST.md](../../docs/VALIDATOR_ONBOARDING_CHECKLIST.md)
2. [GENESIS_ALPHA_COHORT_PLAYBOOK.md](../../docs/archive/GENESIS_ALPHA_COHORT_PLAYBOOK.md) (archived 2026-09-19; if invited)
3. Configure and monitor UTC synchronization with NTP/chrony; live proposals
   outside the protocol's 60-second clock-skew window fail closed.
4. Deploy with `docker-compose.validator.yml` or release `creg-node`
5. Point at `https://api.testnet.cregnet.dev` for registration smoke tests

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
self-contained binaries are mounted at those exact absolute paths. The
generated Python PyPI console launcher is not such an artifact because its hash
does not bind imported verifier/dependency bytes. Empty or mismatched verifiers
must fail closed. A locked one-file build recipe exists, but its output is
platform/architecture/CPython-build specific and is not a reviewed release.
The current Windows Rust launch environment also lacks the private
per-invocation `TEMP`/`TMP`/`TMPDIR` contract required by the frozen
executable, so PyPI activation remains disabled.

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

The validator-local evidence/artifact archive defaults to a shared 10 GiB and
100000-entry quota with a 365-day minimum retention window. Configure
`CREG_NAMESPACE_AUTHORITY_ARCHIVE_MAX_BYTES`,
`CREG_NAMESPACE_AUTHORITY_ARCHIVE_MAX_ENTRIES`, and
`CREG_NAMESPACE_AUTHORITY_ARCHIVE_MIN_RETENTION_SECS` explicitly for each
validator's storage capacity. These are validator-local availability controls,
not consensus parameters, and independently operated validators need not use
identical capacity values. Quota pressure collects only retention-eligible
content and fails closed when it cannot make room; it does not replace backups
or legal retention procedures.

`POST /v1/validator/namespace-authority/attest` and
`POST /v1/validator/namespace-authority/certificates` are implemented
authenticated node interfaces and are registered in the shipped OpenAPI
document. They are not guaranteed to be enabled on the deployed public
endpoint until operators roll out a matching binary and reviewed verifier
artifacts. Both require
`CREG_OPERATOR_API_KEY` through `X-Operator-Key` or
`Authorization: Bearer`; this service ACL is additional to controller and
validator/quorum signatures. Attestation is limited to 2 requests per IP per
minute, with verifier concurrency default 2 and hard maximum 4. Sidecar
`verified_at` is local freshness data and is deliberately excluded from the
shared statement so independent validators sign identical bytes. Treat the
shipped OpenAPI document as authoritative. Real npm/PyPI self-contained
verifier artifacts, official trust material, reviewed policy, a signed
activation spec, multi-operator rehearsal, and independent security review are
still required before testnet activation. See [OPERATOR.md](./OPERATOR.md) for
rollout and rollback.

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
3. Maintainers: [OPERATOR.md](./OPERATOR.md) and [gcp/](./gcp/) scripts (no README yet)
