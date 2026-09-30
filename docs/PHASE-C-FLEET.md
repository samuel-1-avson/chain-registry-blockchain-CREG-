# Phase C real-host fleet (2026-09-25)

## Current fleet (2026-09-26, accepted h6 baseline)

All eight actual R7 candidates run R18 with one signed genesis, four voting
validators, quorum three and four nonvoting observers. All eight passed guarded
replacement/startup. Their h1–h6 hashes, Merkle roots and COMMIT certificates
match. Real h6 browser/CLI committed events and CLI proof verification passed.
V3 terminal work and V4/full committed-state restarts passed earlier; nine dead
letters through h5 are reviewed and preserved.

A new actual h7 owner revocation remains queued only on V1: signed intent
delivery was absent. Prepared crash and two-online-validator recovery acceptance
remain open. Observers also retain stale pending admissions. R19 compiled the
retirement repair; the combined R20 delivery/retirement repair is compiling and
is not deployed yet. Current evidence is in
`testnet-heal-evidence/2026-09-26-phase-c-events-r16/`.

The four additional GCP VMs run real CREG candidates and continue accruing
charges. Every host retains its persistent data and identity; validator signing
journals are untouched. Public explorer/API routing remains on the legacy h0
chain. No public reset or cutover occurred. The dated descriptions below are
historical.

## Historical continuation (2026-09-26, before R12)

The r9 repair image compiled successfully with ID
`sha256:cf9fda37887e0d8c6125280c07bdb36737e495e75d070b80abcc722187180d33`.
The fresh r7 signed spec is verified locally and served at its versioned URL;
the existing default spec is unchanged. The bundled
unverified hash list is excluded. A real pinned OSV snapshot for
`npm:is-number@7.0.0` is ready; its package-version key is present with zero
advisories. The operator added `abusech-malwarebazaar-auth-key` to Secret
Manager in `fiffi-500411`; the snapshot builder fetched 100 unique real
MalwareBazaar hashes without printing or persisting the credential. The source
response SHA-256 is
`59217e0f3351528410efe04580879cdb44ff2322689fb770a34b641aa4733211`, and the
threat database SHA-256 is
`5e3a7beac3c5b7a5d766c30ff024509bc1df23b3f7c5d74349cc54ed6bc960ef`.
The combined scanner profile is pinned as
`648dfd39a02da6dd8c42f2342146208efeae1f435e18d2fcd71317828062fb0f`; its
YARA and OSV inputs match the local private bundle under
`C:\Users\samue\CREG-backups\phase-c-scanner\bundle-r9\`.

The same read-only bundle is mounted on the laptop and all seven GCP hosts.
All eight r7 candidate processes are running the reviewed r9 image and passed
the real runtime readiness capture: matching genesis and scanner profile,
verified spec signatures, four active validators and live P2P connectivity.
All four validators report isolated `nsjail` with no bypass. Correct preserved
r4 P2P identities, including both signed bootnodes, were verified before first
start. Remote Docker versions expose either the image index digest above or
Linux config digest
`sha256:e25b3470e0372b22c5bb98a91cb40a9449051d51cddd366ff8f957cd983f447b`.
The distributed gzip image archive was hash checked on every GCP host.

The real integrity-verified `npm:is-number@7.0.0` package is committed at the
same h1 on all eight nodes. The round needed certified view changes before
finalizing. Retry duplicate suppression and premature local view advancement
remain under review. Certified empty h2/h3 followed; revocation committed at
h4 on all eight nodes. Independent header, Merkle-root and three-signature
COMMIT checks passed for heights 1–4. Real restart checks passed on O1–O4, V1
and V2; V3 restored the chain but retained one queued transaction and forced
entry. V4's restart was deferred after that failure. Full recovery acceptance
is still open. LLM review was explicitly skipped. Prior r4 processes remain
stopped with data and journals preserved. Old V1, V2 and O1 services remain
running, and all four new GCP VMs continue accruing charges while running CREG
candidates. No old-chain reset or public cutover has occurred. Historical
r1–r4 details below are not current acceptance evidence.

These are real, independently provisioned Compute Engine VMs. All four are
running and accrue compute charges. A Phase C candidate was launched on eight
independent hosts, including this laptop, using one signed genesis and a
four-validator quorum of three. The candidate is currently stopped after a
real package exposed a pre-activation namespace validation bug. A repaired
image is building; the old divergent chain remains running and has not been
reset. The candidate's failed-run data and signing journals have been retained.

| Planned slot | VM | Zone | Private IP | Static public IP | Data disk |
| --- | --- | --- | --- | --- | --- |
| V3 | `creg-validator-3` | `europe-west1-b` | `10.96.1.2` | `34.156.122.19` | 50 GiB `pd-balanced` |
| V4 | `creg-validator-4` | `asia-southeast1-b` | `10.96.2.2` | `34.124.165.97` | 50 GiB `pd-balanced` |
| O3 | `creg-observer-3` | `us-west1-b` | `10.96.3.2` | `136.66.249.201` | 30 GiB `pd-standard` |
| O4 | `creg-observer-4` | `africa-south1-a` | `10.96.4.2` | `34.35.191.131` | 30 GiB `pd-standard` |

All four are `e2-medium` Debian 12 VMs in project `fiffi-500411`. Each has a
20 GiB `pd-balanced` boot disk, an independently mounted `/var/lib/creg`
data disk with auto-delete disabled, deletion protection, Shielded VM secure
boot, OS Login, no attached service account, and an active Docker service.
The startup script is `chain-registry/testnet/gcp/prepare-phase-c-host.sh`. The
candidate uses `creg-phase-c-v3`, `creg-phase-c-v4`, `creg-phase-c-o3`, and
`creg-phase-c-o4` containers on these hosts, each with its own private data
directory, P2P identity, and node environment. Validator signing keys are on
V3 and V4 only.

The existing O2 host, `creg-observer-vm` in project
`gen-lang-client-0022105784` (`us-central1-a`), now also has a fresh 30 GiB
`pd-standard` data disk mounted at `/var/lib/creg`. Its old stopped CREG
containers and data were left untouched; its Phase C candidate observer uses
the separate `creg-phase-c-o2` container and data directory and is stopped
pending the repaired image.

## Network and access

- The four new VMs use custom VPC `creg-fleet-vpc`, with one `/24` subnet per
  region under `10.96.0.0/16`. The fleet VPC and the existing project's
  `default` VPC have no active peering. Candidate peers use public bootnode
  addresses and API access restricted by source address.
- SSH to the fleet is allowed only through IAP (`35.235.240.0/20`) and OS
  Login. For example:

  ```powershell
  gcloud compute ssh creg-validator-3 --project=fiffi-500411 --zone=europe-west1-b --tunnel-through-iap
  ```

- Fleet TCP `9000` is open for public P2P. Fleet TCP `8080` is limited to
  the fleet private range, the existing CREG host addresses, and the laptop's
  observed public address by `creg-phase-c-api-old-hosts`.
- A read-only request from V3 and O4 to the existing validator's private
  `http://10.142.0.4:8080/v1/public/chain/stats` returned HTTP 200. This
  checked the earlier network path. The eight-host candidate later reached
  live P2P connectivity and reported four active validators on all hosts.

## Deployment gate

1. The candidate feature build contains exact-block reproposal across
   quorum-certified views and durable prepared-value, view-change, and local
   COMMIT recovery. The ordinary build leaves replacement leadership disabled.
   The candidate still requires real-host acceptance before cutover. No local
   timeout alone may authorize a replacement proposer.
2. Preserve current block hashes, logs, and stopped-node data backups before
   any coordinated reset. The current h1/h2 divergence remains evidence.
3. Create distinct operator-side keys and a single signed genesis with four
   voting validators. Keep the four observer identities outside the voting
   set. Verify the spec signature on every host.
4. Install the reviewed image and real package security pipeline on the four
   validators; start four separate non-voting observer processes. Use real
   package submissions and verify 3-of-4 finality, equal h1/h2 hashes, and
   observer sync before treating the fleet as accepted.

The first candidate spec was **invalid**: it assigned 100 CREG of stake to
each of four new addresses without checking Sepolia. Its signed files were
removed from the local deployment path and the public `/phase-c/` spec endpoint;
hash-identical copies are retained privately under
`C:\Users\samue\CREG-backups\phase-c-candidate\unstaked-invalid-2026-09-25`.
The invalid genesis hash was
`0x5856983bd46a41a04d7f446d007b69f22009756f7ca47bbfa130e6e26486eb80`.
It must never be deployed. The spec generator now reads each validator's live
Sepolia state at the same six-block-deep snapshot and refuses an inactive or
understaked address before writing a candidate. V3 and O4 have distinct
private P2P identities held outside the repository. Both identities were
installed on their real hosts and observed in live peer connections.

The following was the **pre-deployment snapshot** of the old contracts. At
Sepolia block `11781329` on 2026-09-25, the staking contract reported one
active validator and a 100 CREG minimum stake. The four planned addresses were
all state `None`, with zero staked CREG, zero liquid CREG, and zero ETH. The
old spec's validator-1 address alone was active with 100 CREG staked; its
validator-2 address was state `None`. The 2-of-3 governance multisig still
allows emergency admissions, and the treasury holds 495,000 CREG. Four real
applications require 400 CREG plus ETH gas and admission approval. The
existing governance approval script contained two live signer private keys in
tracked source; those keys alone meet the governance threshold. The literals
were removed from the working tree. The old governance and treasury were
retired from the Phase C candidate. The chain reset has not occurred.

## Fresh Sepolia deployment and live candidate

The user chose fresh contracts and keys after the old 2-of-3 governance signer
keys were found in tracked source. Fresh Sepolia governance is
`0x0AB4Db22A9902B8Ef678aBd408889af22277d24E`, staking is
`0x61c00EBe7C46f0bcE63942F7E15Bcc434DFC9B7C`, registry is
`0x23c7FB8e6fd932C1078Ff76244470557E5e6bd99`, and token is
`0xbC2a72107C6765364D0A822a88A6e6C61f30EAa6`. The exact roster and
remaining addresses are in `chain-registry/testnet/phase-c/deployment-manifest.json`
and the signed `chain-registry/testnet/phase-c/chain-spec.json`. All four
validators were funded, staked 100 CREG each, and admitted by real 2-of-3
governance votes. A separate publisher staked 10 CREG. The fresh genesis hash
is `0x6255f69667b13727e618b634993571587adf2101b70ac3dcd73b3fce325ba18a`.

The initial pinned candidate image has OCI index digest
`sha256:c1e2aad2571da92da2a4cd78ba6036fcffacc993cafebf46aea877f40f9a29ac`.
It ran on all eight hosts with four active validators, signed-spec verification,
and a real isolated `nsjail` sandbox. A real npm tarball
`npm:creg-phase-c-heal-20260925@0.1.0` was pinned to IPFS and admitted on V1.
Three validators independently approved it, but h1 did not finalize. The
proposer rejected its retained v4 publisher authorization because the
pre-activation namespace validator incorrectly required an upstream authority
certificate. Certified view changes occurred, but no block was committed.
All candidate containers were stopped with data intact while that bug is
repaired. Do not describe this as successful h1/h2 acceptance or a chain reset.

### Latest repair status (2026-09-26)

The namespace repair was compiled and deployed as image r2 on all eight real
hosts. A fresh 0.1.1 package received three signed pipeline approvals, but no
h1 finalized. The retained 0.1.0 authorization had expired during the rebuild;
it continued to poison proposed batches. A real-laptop diagnostic run recovered
the existing PBFT journal and exposed `publisher authorization is not live at
block time`. Three-vote view-change certificates were recovered successfully;
there is still no block-finality or observer h1/h2 acceptance evidence.

The next repair preserves expired, unprepared publishes in durable dead-letter
storage before removing them from the new-proposal queue and forced-inclusion
tracker. It does not alter a quorum-prepared block. The node image is compiled.
All eight candidate processes are stopped, with logs, data and signing journals
preserved. The old public chain remains running. A new signed chain identity
and fresh data directories are being prepared for the next acceptance run;
previous signing journals must not be deleted and reused under the same genesis.

See `docs/testnet-heal-evidence/2026-09-25-phase-c-candidate-r2/README.md`
for the pinned r2 artifact and preserved run evidence. No old-chain reset or
public cutover has occurred. The four new GCP VMs remain running and billable.

The expiry repair subsequently ran on all eight hosts as r4 with a fresh,
correctly signed spec. A real 0.1.2 publish was accepted and the pipeline
reported VERIFIED, but duplicate transaction variants caused a repeated
publisher-nonce rejection. Direct serialization of a nested HashMap made
transaction hashes change across durable reloads, defeating deduplication.
All eight r4 processes were stopped and their data, signing journals, logs
and final stats preserved. The next node/CLI image is compiling with
canonical transaction hashing, unprepared duplicate/applied-nonce retirement,
and actual block-zero binding to the signed spec. This changes the fresh
chain's consensus encoding and must use fresh storage. See
`docs/testnet-heal-evidence/2026-09-26-phase-c-candidate-r4/README.md`.

The project has a US$200 monthly **alert-only** budget at 50%, 80%, and 100%
gross spend. The four VMs are running and accrue compute charges; their disks
and reserved IP resources remain provisioned. The budget covers only
`fiffi-500411`; the existing CREG project and laptop are outside its scope.
