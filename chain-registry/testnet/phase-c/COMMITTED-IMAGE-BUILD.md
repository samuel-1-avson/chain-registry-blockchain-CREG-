# Phase C image from committed source

Run from the repository root in PowerShell to prepare a clean source context:

```powershell
$phaseBuild = './chain-registry/testnet/phase-c/build-committed-candidate.ps1'
& $phaseBuild -Revision '<reviewed-full-commit-sha>'
```

The default `node-runtime` target is for a node-only repair. Use
`-Target node-cli` when a change also requires rebuilding the CLI.

Preparation does not run Docker or change a live system. It exports only
explicit tracked compiler inputs with `git archive`, excludes working-tree
files, verifies the signed R7 chain spec during compilation, and records hashes
for the inputs, Docker recipe, Cloud Build YAML, helper, builder/runtime pins
and Cargo lockfile. It saves the source context and build arguments outside the
repository for review.

For the bounded remote node-only build, read `node-runtime-build.json` in the printed
evidence directory and use its `source_context`, `source_revision` and
`source_manifest_sha256` values:

```powershell
$context = '<source_context from node-runtime-build.json>'
$record = Get-Content '<node-runtime-build.json path>' -Raw | ConvertFrom-Json
$manifest = Get-Content (Join-Path $context 'BUILD-SOURCE-MANIFEST.json') -Raw | ConvertFrom-Json
$image = 'us-central1-docker.pkg.dev/gen-lang-client-0022105784/creg-testnet/creg-node:phase-c-rpc-r31-rust193-' + $record.source_revision.Substring(0, 12)
gcloud builds submit $context `
    --config (Join-Path $context 'cloudbuild-node-candidate.yaml') `
    --project gen-lang-client-0022105784 `
    --region us-central1 `
    --substitutions ("_IMAGE=$image,_SOURCE_REVISION=$($record.source_revision),_SOURCE_MANIFEST_SHA256=$($record.source_manifest_sha256),_SOURCE_DATE_EPOCH=$($manifest.source_date_epoch)")
```

The Cloud Build uses the default `e2-standard-2` worker (2 vCPU, 8 GB RAM),
100 GB worker disk and a 2 hour timeout. At the published $0.006/build-minute
rate, the compute ceiling is $0.72 before any remaining monthly free allowance;
the build stops at the timeout. The first 100 GB of worker disk is listed as
free. The completed candidate is pushed only to the private project Artifact
Registry. It does not update running nodes or DNS. [Cloud Build pricing](https://cloud.google.com/build/pricing).

The previous combined node-and-CLI build completed the node compile in about
89 minutes, then timed out during the unchanged CLI build at the two-hour cap.
The node-only target removes that second compile and keeps the same $0.72
maximum compute charge.

The recipe pins Rust 1.93.0, the Ubuntu 24.04 builder and accepted secure
Artifact Registry runtime by digest. Cargo uses `--locked`; the node enables
`phase-c-real-fleet`, and the CLI enables
`common/phase-c-canonical-transactions`. Both retain the Phase C signing public
key `3fe87833177b04b0a18600eace041692a35fb2b85f6e14e6734d894a0fd61c01`.
The CLI validates the committed signed R7 spec offline as part of compilation.
The embedded Vite build is disabled for this node feature set.

The runtime uses
`us-central1-docker.pkg.dev/gen-lang-client-0022105784/creg-testnet/creg-node@sha256:ca4d04e77e9680cb878800f2ea72c1ce7dac6bb3575a55c9921a7314975fb1ef`.
The `node-runtime` target replaces only the node executable; the `node-cli`
target replaces both node and CLI. Each image carries the source manifest and
source labels. Cloud Build checks the applicable version command and logs the
binary hashes before pushing the candidate tag.

The worker needs access to Docker Hub, Ubuntu package repositories, Cargo
dependency downloads and the existing private Artifact Registry runtime. The
target is `linux/amd64`. Compilation uses one job, with an isolated cache keyed
to the full source manifest. A cold RocksDB, ONNX, WASM and Rust release build
can take substantial time. This pins source and compiler inputs, but not
every downloaded Ubuntu/native artifact; it does not promise identical binary
hashes across separate rebuilds. The CLI target is built for the Ubuntu 24.04
runtime, not the separate Ubuntu 22.04 portable release.
