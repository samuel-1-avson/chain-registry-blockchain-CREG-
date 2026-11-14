# Phase C real-node run r4

The expiry repair compiled and ran on four real validators and four real
observers across the laptop and seven GCP VMs. The pinned OCI index was
`sha256:ca4d04e77e9680cb878800f2ea72c1ce7dac6bb3575a55c9921a7314975fb1ef`;
Linux tar-loaded images reported the corresponding config digest
`sha256:6c6861bc4c8efc6870cea646abc880c383cff1fb57267c699438844611ff543d`.
The image tar SHA-256 was
`98cbb61b0a0346b88bef113772edce0170741b5e6d9b1bf788f0063ba866634c`.
The node binary SHA-256 was
`10309c933367a5193a30ee2279e0c3bf6d27ecfafd788593a2618c3b41a5f8bf`;
the real nsjail binary was unchanged from the secure r2 runtime.

Before the fresh run, a recovery process on the real laptop opened the old
candidate data and recovered its three-vote view-change certificates. It
preserved both expired publishes in dead-letter storage; the prior namespace
expiry rejection did not recur during that short recovery observation. The
process was stopped. Its full log is retained privately as
`C:\Users\samue\CREG-backups\phase-c-candidate\v1\run-r4-recovery.log`.
This was a recovery observation, not a block-finality or failover pass.

## Fresh signed spec and package

The fresh signed chain identity was `creg-phase-c-testnet-2026-09-26`, with
signed-spec genesis hash
`0x13677af7d23547816190c0d98704abaf1daf793d444fefae838b14a89c285a67`.
All eight hosts verified its signature and reported four active validators
with 400 CREG total stake. Observed peer counts were 1–7. Their actual block-zero
hash remained the historical constant
`068364cf778ede5a0b1c814c59b0314f9a8598a959fa5a8e08ea78bc1b296b53`.
The signed-spec identity and actual block-zero hash are distinct values.

A real npm tarball, `npm:creg-phase-c-heal-20260925@0.1.2`, was pinned to IPFS
as `QmSX7dp6AMRZseQwEEgcqbbcJHERWJ2L2tvtcyxi2ZPRGD`. Its new publisher
authorization bound the correct signed spec and was submitted at approximately
04:45 UTC on 2026-09-26. V1 returned HTTP 202. The pipeline reported VERIFIED,
and three-vote view-change certificates formed. No h1 finalized, and the
revocation was not submitted.

The proposer exposed a new error: `publisher authorization nonce has already
been applied`, at block transaction index 1. Its buffered batch grew from
seven to nine transaction variants. Source review found that
`transaction_hash` and `merkle_root` serialized records containing a nested
`HashMap` directly. Reloading a durable record can change map iteration order,
changing its hash and defeating the queue's hash deduplication. The repair
uses sorted-key transaction encoding consistently for work IDs and Merkle
leaves, and preserves already-applied or duplicate unprepared authorization
nonces before retiring them from proposals.

The next candidate also binds its actual block zero to the verified signed
spec. r4 used a constant genesis block, and its safety journal was initialized
with that actual block hash. Changing only the signed chain ID therefore did
not give view-change signatures a distinct parent domain. The next encoding
is gated by the explicit Phase C build feature and requires fresh storage;
do not upgrade r4 data or rewrite its signing journals in place.

## Preserved state and build status

All eight r4 candidate containers were stopped with their data and PBFT
journals preserved. Full logs, data archives and final stats are retained on
each real host and under `C:\Users\samue\CREG-backups\phase-c-r4\<slot>\`.
The next node and CLI image is compiling. No old public-chain reset or public
cutover occurred. The four new GCP VMs remain running and billable.

A PowerShell variable collision briefly generated a spec with Sepolia's chain
ID in the CREG chain-ID field. It was detected before any node launched with
it. Invalid spec and environment files were preserved privately, the variable
was renamed, and the corrected spec was signed and verified before launch.
Separate local Docker cleanup also deleted the compiled image and builder
tag. Docker events recorded the deletions; the operator agreed to pause the
cleanup. The runtime was restored from its SHA-256-verified image archive.

L1 reward transactions and complete ML detection coverage remain unverified.
The bridge key is unset; a successful reward-hook log is not an L1 receipt.

## Private archive checksums

| Slot | File | SHA-256 | Bytes |
| --- | --- | --- | ---: |
| v1 | run-r4-data.tar.gz | 1abf97e20eed74a9aa38d00d3cb7889af4f27071c288cdee979737ef617b860a | 31249 |
| v1 | run-r4-stats.json | 76a54797784f77534d2527e0813ebeb7c84f7f68a03c7743aeaa628b970492d4 | 853 |
| v1 | run-r4.log | 592cbdb50fe747dc4b1cd30a5679ffa299590dab831bd8cc351fa33a121449bc | 395635 |
| v2 | phase-c-r4-v2-data.tar.gz | 63cbe9b9c720ddabf3059f090176da46e8cedf827e6ec89b432255860d7d1db8 | 31002 |
| v2 | phase-c-r4-v2-stats.json | 4461d0c977f5514b53faae58aa79cdbabf33184dfe515eb4bbbf0dc6f87ab8bb | 736 |
| v2 | phase-c-r4-v2.log | 6dd781e33ad3b6a62f87e0ca7b52d9be0cdc7076e708825af03639aef38ca33d | 107860 |
| v3 | phase-c-r4-v3-data.tar.gz | 7750df8ba8954c0c2ba17d6ae37b3e10f9a841cf6c9b7034a43a3b88ea1e9381 | 31330 |
| v3 | phase-c-r4-v3-stats.json | 383e3c2d8380af8aec65f846a912f8202aeeeec012925838dfa6b47fcb27b17b | 736 |
| v3 | phase-c-r4-v3.log | 4ff3e85801e8d656180c25b1908ec2a65169b6da56ed1a50a6ad5dee83164ad9 | 221459 |
| v4 | phase-c-r4-v4-data.tar.gz | 332104106eea55877ce7f67f513a66ce6f67c0ae0a59ed658cca87886b6cce10 | 29744 |
| v4 | phase-c-r4-v4-stats.json | f1f15099e3f85d1245e295dab639d38ec73c2cbde15ecf347f666b9195c52aae | 737 |
| v4 | phase-c-r4-v4.log | 18e5d66b3188d15b3a282439505c7c7f31f37ed4395960b4b1587a8ea06960ad | 150774 |
| o1 | phase-c-r4-o1-data.tar.gz | d68d91751f31393c9ffdeb9ccfd8ff99d788711b6bb5bef7141529866ffff495 | 21304 |
| o1 | phase-c-r4-o1-stats.json | 8749ec4c33de960383eb53d2a5256b0f58de1d484c96f5d4d8eae078fde390eb | 737 |
| o1 | phase-c-r4-o1.log | ac5153774c5932caa43b4dbc375b97ea0ed07370b4be08769e2405c605fa541e | 69993 |
| o2 | phase-c-r4-o2-data.tar.gz | d6999fccc358f6939aa723361d703c8669566ab8b186db05a91380e12e825f9c | 21226 |
| o2 | phase-c-r4-o2-stats.json | bb332c9a9b37c64564c379a6798cc9741aa6cab36885d11e49f2aabf043ef1bb | 737 |
| o2 | phase-c-r4-o2.log | 19da4f76c19322f76fbdcf56e9075d5cd7e1a7c081c9843777365c1c62ab810e | 69741 |
| o3 | phase-c-r4-o3-data.tar.gz | 893f5c975f304163ef5471b3924557be6bd6ad758cb8b9baeb99e11c90f79581 | 21383 |
| o3 | phase-c-r4-o3-stats.json | 11419901193a37cd0058e2ef676b4a6017ee9163600a96b68290fd6bc29f90d7 | 737 |
| o3 | phase-c-r4-o3.log | cb4c27f68a95dbc8eaccf29a47b69b94cf38a22a0141bdfd052be28f77e3f295 | 78388 |
| o4 | phase-c-r4-o4-data.tar.gz | 438fa28f4939eb61194444383ecb469869bfb97d6e5134ce08b10f205181563f | 19906 |
| o4 | phase-c-r4-o4-stats.json | c9ca61e70fb821b87bb1c0ea01cb01abbf57bf6bc00b8110718f547f7b60adfd | 737 |
| o4 | phase-c-r4-o4.log | a278969498745094f9ff7c1866a4c91683b0bd8a21c1cfb0fa4cdc1dc2eaef84 | 191953 |
