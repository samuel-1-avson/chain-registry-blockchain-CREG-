# W2 — Real CLI publication and event acceptance

**Result:** Passed on 2026-09-27. This is a real npm artifact, real IPFS pin, real publisher signatures, live four-validator consensus and the eight real candidate nodes. No fixture or simulated response was used.

## Chain and artifact

- Chain: `creg-phase-c-testnet-2026-09-26-r7`
- Signed spec identity: `1852b9a86dd3b53e7b2de4a774723a4fe48e862f9c61d14e5917b42b21055234`
- Starting shared tip: h12, `a329eb216ce1fd18e1f5510333cb683db34ca6e789b5a3ef229dabe2af08fc7d`
- Published package: `npm:kleur@4.1.5`
- Upstream tarball SHA-256: `0d99562acf82fca4afce2a57b687289a524ac6755dcaf1fdf82a147473091469`
- IPFS CID: `QmSEhmMVab63QT8AhTvqwXozKEj3PkQG5vE95nzs8PAju8`
- R11 OSV snapshot epoch: `osv-2026-09-27-phase-c-kleur-r11`; five real package/version queries, zero OSV advisories
- Scanner profile digest on all eight nodes: `f397fac29df29438f0ee15580547def5fe66a03af60153d22993f4a7af9fce8c`

## Finality and event delivery

- Finalized publication block: h13, `8448cce76d7258f051cf9632d2ef7181da1321f5d19a3b5d00f00f8975aa2993`
- Previous hash: `a329eb216ce1fd18e1f5510333cb683db34ca6e789b5a3ef229dabe2af08fc7d`
- Merkle root: `54d76b0f1255b554395bf077365e011f5ea0dca3dabc40bb485ae26fa4cba568`
- Proposer: validator-4. COMMIT certificate: three distinct validators (validator-2, validator-3, validator-1). Four validator vote events were captured.
- CLI watcher recorded submission, all four votes, block production and package verification.
- Browser watcher recorded `package_submitted`, four `validator_voted`, `block_produced` and `package_verified`; its stream opened, rendered the package/event names and reported no page errors.
- All four validators and four observers report the same h13/hash and package count 5. All report the R11 digest; all are ready with clear recovery queues and no active PBFT rounds.

## CLI proof and scanner result

- The R7 chain spec and detached signature validated with the actual release CLI.
- A locally rebuilt release CLI embeds the signer public key from that validated R7 spec. `creg verify kleur@4.1.5 --ecosystem npm` passed Merkle inclusion verification against the signed-spec genesis checkpoint.
- `creg status` reports the package as `Verified`, and the API proof response carries the canonical package authorization and the three-signature block certificate.
- The scanner score is 55 (`elevated`, `review`), with zero OSV advisories, a medium deep-scan warning and low environment-read warnings. Kleur's upstream source reads `FORCE_COLOR`, `NODE_DISABLE_COLORS`, `NO_COLOR` and `TERM` to control terminal colors. The static pattern review found no network imports, child-process spawn or filesystem writes. Keep the scanner label and skipped LLM stage visible in acceptance records; validator approval does not erase those findings.
- No package installation or revocation was performed as part of W2.

## Evidence files

- [Fleet convergence](publication-fleet.json)
- [Browser named events](browser-publish.json)
- [CLI publish output](cli-publish.log)
- [CLI live event output](cli-watch-publish.log)
- [Node package Merkle proof and canonical authorization](package-proof-api.json)
- [Finalized block and COMMIT signatures](block-13-api.json)
- [CLI Merkle proof result](cli-package-proof.log)
- [CLI canonical record](cli-chain-record.json)
- [R11 package provenance](package-provenance-r11.json)
- [Pinned CLI build identity](cli-build-r7-signer.json)

The CLI did not save its in-memory HTTP request body as a separate file. The finalized transaction proof is the preserved source for the exact canonical authorization fields and signatures. No private signing key or Secret Manager value is included in the evidence.
