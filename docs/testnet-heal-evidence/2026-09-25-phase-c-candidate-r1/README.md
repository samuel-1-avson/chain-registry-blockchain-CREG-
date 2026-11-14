# Phase C eight-host candidate, first live run

Observed 2026-09-25 UTC. This was a real deployment on four validator hosts
and four observer hosts, not a simulation. The old divergent chain was left
running on its original containers and its pre-reset archives are preserved.

## Identity and deployment

- Signed candidate chain ID: `creg-phase-c-testnet-2026-09-25`.
- Signed spec genesis hash: `0x6255f69667b13727e618b634993571587adf2101b70ac3dcd73b3fce325ba18a`.
- Block-zero hash observed on all eight nodes: `068364cf778ede5a0b1c814c59b0314f9a8598a959fa5a8e08ea78bc1b296b53`.
- Secure image OCI index digest: `sha256:c1e2aad2571da92da2a4cd78ba6036fcffacc993cafebf46aea877f40f9a29ac`; its Linux Docker config ID is `sha256:9e596fc83aed136253d25706651eaaf355a8002698f125ba189f175bed420da6`.
- All eight nodes reported `spec_signature_verified=true`, four active
  validators with 400 total CREG staked, and live P2P peers. The four
  validators reported quorum three and `nsjail` isolated with
  `dev_bypass=false`.

## Real package result

The publisher staked 10 CREG on fresh Sepolia Staking and signed
`npm:creg-phase-c-heal-20260925@0.1.0` with a fresh Ed25519 key and its
staked EVM key. The real npm tarball SHA-256 is
`b42e4347aabe6c038de56ff1b1083d079df4da1aa773bcf1037fd2aaf5b4671b`.
It was pinned to IPFS CID `QmbLKvCFFUeKgk5tP7bkiEwH7cUY32cufyffv2vnVFGd6r`.
V1 returned HTTP 202 for the signed request. Its pipeline completed a
three-validator approval and queued a v4 authorized publish transaction.

H1 did **not** finalize. Proposers logged `refusing to propose a block with an
invalid namespace transition`; source review traced this to
`namespace_consensus::validate_authorized_publish` requiring a first-claim
upstream authority certificate before CR-07 activation, while admission
accepted the same v4 request without one. Certified view changes at h1
occurred, but no block was committed. Candidate containers were stopped and
their data and signing journals preserved. Do not count this as h1/h2 or
observer-sync acceptance.

## Private archive digests

Private logs and signing-state archives are at
`C:\Users\samue\CREG-backups\phase-c-candidate\<slot>`. Do not publish these
archives; validator data includes signing material.

| Slot | Log SHA-256 | Validator data SHA-256 |
| --- | --- | --- |
| V1 | `5883d47d3650745138b02ae254929f673681de54b3c238cdc0b25e576effcc31` | `df03f975bfb7be3529a4827e99a92029d86df6c8abf1542bbe9f47926aef3c79` (zip) |
| V2 | `7fefe0af935ba3e126e515ce26d726ae7b9a84726eb032a99088a60b68594b16` | `56a24a9db19a1e4701a41e760796acee2ab16c2d6b8c8f71347f56236f246993` |
| V3 | `d7efd21d62bd2a81b1519cd3fee74def463143792f1da04996a6383c6c166ea2` | `f557dcd26b067a06f5729760b81c0c5c8dab0d9307cbe13b7dad30a6b18f1d0e` |
| V4 | `af507352496108764de682174467cfa19597ca91108eabdee3f8ddb2db265c6b` | `838ce6d5b0b16ed9a0ef7ad8a051b7501b483315fb09e21949ff7030d3986065` |
| O1 | `3161fc1db2fbd57774e4dab787a06ff3c9268780cab29009621e286a81d52bbd` | — |
| O2 | `03f0cbe8f17e338b2784810d4a8323e5edca808dba65137832a1b8bf558c9fa7` | — |
| O3 | `79d6bbf731d9ecbdabe6b61e9921478c58bc1815da714d3144085d30529a4bde` | — |
| O4 | `d32adbc88a9f819f3fb0d7b4c49edf83fd33f9a85d4603c812402f6136db5886` | — |
