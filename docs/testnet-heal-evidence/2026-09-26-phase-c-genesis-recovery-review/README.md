# Phase C reviewed genesis/recovery/client build

Source review: `../../../PHASE-C-GENESIS-RECOVERY-REVIEW.md`.

- Candidate tag: `creg-node:phase-c-secure-r7-review`.
- `source-manifest.json`: SHA-256 of 585 input files, frozen before dispatch.
- `source-manifest-initial.json`: the initial frozen review, preserved before
  adding the final mixed-profile startup guard. Revision 2 changes only
  `crates/node/src/main.rs` and was frozen before re-dispatching the queued build.
- `source-checks.json`: Rust and PowerShell parsing and whitespace results.
  Also records successful Phase C common/resolver/consensus compiler checks.
- `build-status.json`: current queued/compiling/built/failed state.
- `build-phase-c-secure-r7-review.log`: generated when Docker compilation starts.
- `image-build.json`: generated only after successful compilation and a second
  source-manifest check. Records the actual local image ID.
- `image-archive.json`: preserved Docker archive path, size and SHA-256. The
  local archive is `C:/Users/samue/CREG-backups/phase-c-reviewed-r7/phase-c-secure-r7-review.tar`.
- `chain-spec.json`, `chain-spec.json.sig`, `spec-preparation.json`: the local
  fresh r7 signed spec, verified signature, source digests and real Sepolia
  roster anchor. Prepared without publishing or starting nodes.

The preceding r5 build has an earlier source snapshot. The original r6 review
manifest is retained in the sibling `2026-09-26-phase-c-source-review` directory;
changed inputs must make that wrapper stop before building. The r7 wrapper
waits for the existing r5 Docker process so dependency-cache work is serialized.
Docker cache and preserved images must not be pruned during these builds.

No automated tests, candidate deployment, chain reset, publication/revocation
acceptance or failover experiment was performed as part of these source checks.
Successful compilation is separate from independent real-host acceptance.

## Completed artifact checks

The r7 node and CLI release build succeeded. The reviewed CLI accepted the
fresh r7 spec with the pinned signer and signature required, reporting four
validators and actual block zero
`0f2a3b109318f0979807f3d921f72775b2a37ce4eb28415a62d9f53b3766eed4`.
The image ID is
`sha256:54d0bc72f2b13497443befe12e75356e675ceb19329e66ceef72fa7bd200fce1`.
The archive SHA-256 is
`d61d83871aa7f0e7ec353b2b0ee9aea396b17512afd9ad85b14fa1fd0f1fd16f`.
All 585 reviewed file hashes still matched after compilation and archival.
The signed spec is local and unpublished; public cutover remains pending.
