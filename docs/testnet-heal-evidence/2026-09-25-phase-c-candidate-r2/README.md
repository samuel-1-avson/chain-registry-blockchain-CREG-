# Phase C candidate, repaired image run r2

This was a live run on four independent validator hosts and four independent
observer hosts. No mock nodes or small-cluster quorum override were used.
The signed chain ID was `creg-phase-c-testnet-2026-09-25` and the signed spec
genesis hash was
`0x6255f69667b13727e618b634993571587adf2101b70ac3dcd73b3fce325ba18a`.
All eight nodes reported the same block-zero hash,
`068364cf778ede5a0b1c814c59b0314f9a8598a959fa5a8e08ea78bc1b296b53`,
four active validators, and a verified spec signature. V1 and V2 reported an
isolated `nsjail` sandbox with the development bypass disabled. P2P peers were
observed across the fleet.

The repaired image was built with the pre-CR-07 namespace claim fix. Its OCI
index digest is
`sha256:71eff57882c0f7f11377430a5a84d1a7773786b49cc654020381c235443b0beb`;
the Linux image config digest is
`sha256:3862a04b4a9e8d8b101b9713a80bbad75ddc718af21208004718ccbfaca43e1b`.
The private OCI tar export has SHA-256
`a98af88532fb0dd5a3151ecbbe827ab66f365419c0b5070bdf356cd0327f2afc`.
Its `nsjail` binary matched the first candidate byte for byte; the node binary
changed. The three existing GCP hosts pulled the pinned registry digest. Four
new VMs received the tar over IAP and matched its SHA-256 before loading it.

The previously submitted real package `npm:creg-phase-c-heal-20260925@0.1.0`
had expired while the image rebuilt. A real, newly signed and IPFS-pinned
`npm:creg-phase-c-heal-20260925@0.1.1` request returned HTTP 202 on V1. Its
pipeline completed real analysis and collected three signed approvals, then
reported VERIFIED. The producer still logged an invalid namespace transition
and repeatedly retained failed non-durable work in memory. No h1 finalized on
any host. The exact nested validation cause was not included in the r2 log.
A diagnostic build on the real laptop recovered the same signing journal on
2026-09-26 and exposed the cause: `publisher authorization is not live at block
time`. The producer repeatedly retried expired, retained publish transactions.
Recovered view-change certificates each had three votes and quorum three.
The diagnostic process was stopped and its full log preserved privately as
`v1/run-r3-diagnostic.log`. This diagnoses retained expired work; it does not
establish block finality or a passing failover run.
A later renewal of the 0.1.1 request was signed
privately but not submitted.

All eight r2 candidate containers were stopped without deleting their PBFT
journals or changing the signed genesis. The old public `genesis2` containers
are still running; **no old-chain reset or public cutover occurred**. The four
new GCP VMs remain running and accrue compute charges. Private r2 data and
full logs are held under
`C:\Users\samue\CREG-backups\phase-c-candidate\<slot>\`.

## Private archive checksums

| Slot | File | SHA-256 | Bytes |
| --- | --- | --- | ---: |
| v1 | run-r2-data.tar.gz | 454c228d4048ebc056d0f2a864ad517e3b823778227521ac7baceb26983a03b4 | 57124 |
| v1 | run-r2.log | b731f1e8c2770afe7280e013dd1dbf3c51220617470885aaf16a0da47edeb5aa | 803711 |
| v2 | run-r2-v2-data.tar.gz | 333ff12790656e4a31d19c2d3c2af38a7a2a94237a410c37f5022e9f39839b57 | 58722 |
| v2 | run-r2-v2.log | 84f5a26b45c30d99e01f9363e7a5e7eaac37af6d116ea6974b35a5d36f0af15c | 129646 |
| v3 | run-r2-v3-data.tar.gz | cd062efed65bbd909d137dfd72e92a7108d11e67cabee89037de9f4af98a0b82 | 61521 |
| v3 | run-r2-v3.log | 813ac763c8516cc31ca4002f0f08a834a64f05f544ab790dd3dbef7d912e4f3a | 131506 |
| v4 | run-r2-v4-data.tar.gz | 7b0b593893ec6be9f2192de68ba9c226023d255c656f6cae959a457b31300178 | 58624 |
| v4 | run-r2-v4.log | b40e4cfcf2339069a454026f02704f3ab84dc18490eb5618273a078bf0ba7f18 | 103156 |
| o1 | run-r2-o1-data.tar.gz | f1919d6df114e6772dd7b664266b05ad14b8d332b68c910e18a06474df75e9b8 | 40460 |
| o1 | run-r2-o1.log | 14a3450853134386f8040d97ab10ea31fbc2a247237eaefd9a6c295ee24e30f0 | 93965 |
| o2 | run-r2-o2-data.tar.gz | e0d4bc902638bd998b1820ff2d51a816ddb7dd4c90452d05bd0b946c044bb38e | 40895 |
| o2 | run-r2-o2.log | 03148b580d8a8b964f1ee69978feef468b7c0c6d9e32a358de9d5f82cc487ab8 | 97565 |
| o3 | run-r2-o3-data.tar.gz | 95cfdb11f8a569a5b49a34056d9f6b4830838b735497996f9b12898a7e51dcd0 | 39644 |
| o3 | run-r2-o3.log | d0de075f5bd74cde8bc7455de38ac58e55b87ce9b0454651a48639e6fcaaa95f | 53020 |
| o4 | run-r2-o4-data.tar.gz | baf7848eafdd9f486ae521acd1cd86ba1beeaec272d9547aae435d179c9658b4 | 39496 |
| o4 | run-r2-o4.log | 2a58926cc10158e9a695034992db5ee802ead4fea644e00ec1dd40873a004668 | 141994 |
