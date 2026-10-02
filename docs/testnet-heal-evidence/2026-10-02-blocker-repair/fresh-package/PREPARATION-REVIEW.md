# Fresh real package preparation review

Prepared `npm:has-flag@4.0.0` on 2026-10-02T08:10:35.499Z for a later R7 acceptance run. The existing parameterized `prepare-next-real-package.cjs` helper downloaded the exact [official npm metadata](https://registry.npmjs.org/has-flag/4.0.0) and [registry archive](https://registry.npmjs.org/has-flag/-/has-flag-4.0.0.tgz) and preserved the genuine OSV query/response. No package files were extracted, installed or executed; no publisher signature, IPFS pin, chain submission or scanner deployment was performed.

| Evidence | Value |
| --- | --- |
| Archive | `package.tgz`, 2206 bytes |
| Archive SHA256 | `77a7eb1411d927bb8a5ca7069dbe168886d63c88f446f0b81500a2cf23ddb5b1` |
| Registry SHA512 integrity | `sha512-EykJT/Q1KjTWctppgIAgfSO0tKVuZUjhgMr17kqTumMl6Afv3EISleU7qZUzoXDFTAHTDC4NOoG/ZxU3EvlMPQ==` |
| Registry metadata SHA256 | `0a45ddada39d4474160dbf7234843ef3d76afa404b9287dab32d92be0d734345` |
| OSV response SHA256 | `44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a` |
| OSV advisories at capture | 0; actual preserved response is `{}` |
| License | MIT |
| Archive inventory | Five regular files, 4,419 unpacked bytes |

`archive-static-review.json` records every file's size/hash and independent SHA512/SHA1 checks against the registry. The archive contains `package.json`, `index.js`, `index.d.ts`, `license` and `readme.md`; the inventory exactly matches registry file count/unpacked size. There are no install or publish lifecycle hooks, runtime dependencies or optional dependencies. The only declared script is the developer test command; it was not run. The 330-byte CommonJS implementation checks a supplied argv array, defaults to `process.argv`, and compares a flag's position with the double-dash terminator. Static inspection found no network, filesystem-write or process-spawn calls.

The real eight-node absence capture completed at 2026-10-02T08:13:36.7092545Z. V1-V4 and O1-O4 each returned HTTP 404 for this exact canonical and reported signed R7, finalized/tip height **20**, hash `730b01c1a67a4796a16f599169e4ebf77e01d2e8ce92d1e4fb3715a7c4354285`, four active validators, correct role, and scanner profile `7e1d95f9c8423778211944a1dea46674d8cbd3f0aee3b0eea6b5250f11d009cd`. The independently reviewed actual genesis is `0f2a3b109318f0979807f3d921f72775b2a37ce4eb28415a62d9f53b3766eed4`. Raw route evidence is preserved in `package-absence-at-h20.json`; `chain-absence-review.json` pins that file's hash and records the eight results.

Preparation and package absence pass at these capture times. This does not establish full fleet acceptance: the current scanner profile covers the eight historical versions and lacks `has-flag@4.0.0`. A new pinned snapshot/profile based on the genuine captured OSV response must be reviewed and deployed to all eight nodes. RPC provider quorum, current validator-set sync, reviewed runtime images and full readiness remain separate gates. Recheck absence/readiness immediately before fresh publisher authorization and submission. Zero OSV advisories and static review do not replace actual validator scanning, three-validator approval/finality certificates, identical new block hashes on all eight nodes, named event delivery or positive CLI proof.

Evidence files: `registry.json`, `package.tgz`, `osv-query.json`, `osv-response.json`, `review.json`, `archive-static-review.json`, `package-absence-at-h20.json`, `package-absence-helper.log`, `chain-absence-review.json`. `preparation-evidence-manifest.json` pins the saved evidence and the existing preparation/capture helpers. Historical helpers and old evidence were not edited.
