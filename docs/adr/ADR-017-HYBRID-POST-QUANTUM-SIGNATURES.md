# ADR-017: Hybrid Ed25519 + ML-DSA-44 signatures

- **Status:** Proposed (2026-11-21). Approach (hybrid, not replacement) chosen by the maintainer on 2026-11-21. Increment 1 (the scheme abstraction, no behaviour change) is implemented; see section 10.1. The open decisions are in section 9.
- **Context/problem:** every CREG-native signature is Ed25519: PBFT votes, package votes, view changes, validator-set transitions, the chain spec, weak-subjectivity checkpoints, the transparency log and its witnesses, publisher and namespace-controller signatures, revocations, and threshold-decryption shares. A large enough quantum computer breaks Ed25519 by recovering the private key from the public key, and every validator and publisher public key is public. Today Ed25519 and ML-DSA-44 offer about the same classical security (roughly 128-bit; ML-DSA-44 is NIST category 2). The reason to migrate is quantum resistance, not classical strength.
- **Options:** (a) keep Ed25519; (b) replace Ed25519 with ML-DSA-44; (c) hybrid: every signature carries both, and both must verify.
- **Decision:** (c). Ed25519 protects against a bug in the young ML-DSA implementations; ML-DSA-44 protects against a quantum attacker. Forging a hybrid signature needs both broken. Scheme identifier `hybrid-ed25519-mldsa44-v1`. Rolled out behind a consensus activation height, following the CR-07 activation pattern.
- **Security implications:** see section 7. The construction must be non-separable (section 3), the two keys must be generated independently (section 4), and ML-DSA library upgrades become consensus changes (section 6).
- **Operational implications:** every validator gets a second secret, `CREG_VALIDATOR_PQ_KEY`, kept with the same custody as its Ed25519 key. Consensus messages grow about 40× (section 5).
- **Migration implications:** history before the activation height stays Ed25519-only forever and must stay verifiable. Pre-activation history is protected against a future quantum forger only by hybrid-signed checkpoints taken after activation (section 7).
- **Rejected alternatives:** section 8.
- **Revisit conditions:** NIST or IETF guidance changes the recommended parameter set; an independently audited Rust ML-DSA implementation ships; the validator set approaches the certificate-size limit in section 5; a standardized post-quantum VRF or aggregate signature appears.

## 1. Inventory

Measured on `main` at `9bfd1dc`.

| Area | Ed25519 use |
|---|---|
| Rust | 43 files, about 91 `ed25519_dalek` references: cli, common, consensus, node, resolver, threshold-encryption, validator, zk-validator. |
| JavaScript | About 20 files: explorer, `scripts/validate-cr07-activation.mjs`, `scripts/verify-weak-subjectivity-checkpoint.mjs`, `scripts/validate-consensus-evidence.mjs`, `testnet/` tooling, and the npm-verifier sidecar. |
| Pinned wire formats | `consensus/tests/signed_message_formats.rs` pins `creg-vote-v2` and `creg-pbft-v1`; `node/src/gossip.rs` also defines `creg-vote-v3`. |
| Schema | `ValidatorSet` entries require a 64-character lowercase Ed25519 `pubkey` (`common/src/validator_snapshot.rs:138`). `BlockSignature` holds a hex Ed25519 `pubkey` and `signature`. The chain spec's `signing.signature_algorithm` must be `ed25519`. |
| Contracts | `PrivateRegistry.sol` stores secp256k1 encryption keys (unaffected). `ZKSlashingVerifier.sol` and `zk-validator/src/circuits.rs` take the Ed25519 public key as two 128-bit limbs; the `DoubleSignProof` circuit is retired (ADR-016). |

## 2. Scope

| Signature | Moves to hybrid | When |
|---|---|---|
| PBFT PREPARE and COMMIT (`BlockSignature`), view-change votes, reproposal proofs | Yes | At the activation height `H` |
| Package votes (`vote_accumulator`, gossip) | Yes | At `H` |
| Validator-set transition signatures | Yes | At `H` |
| Weak-subjectivity checkpoints and validator snapshots | Yes | From `H`; the first hybrid checkpoint covers all earlier history |
| Chain spec | Yes | The next signed spec; `signature_algorithm: "hybrid-ed25519-mldsa44-v1"` |
| Transparency-log tree heads and witness signatures | Yes | After consensus is stable on hybrid |
| Threshold-decryption share responses | Yes | With the transparency log |
| Publisher, namespace-controller and revocation signatures | Accepted from `H`; required after a grace period | Publishers are external and need time (decision 3) |

Out of scope:

- **VRF** (`consensus/src/vrf.rs`). It uses `sha256(signature)` as its random output, which requires each signer to have only one valid signature per message. ML-DSA signatures are not unique, so they cannot serve this role. The VRF keeps using the Ed25519 half of the hybrid key. Ed25519 has the same weakness: verification does not check that the nonce was derived deterministically, so a proposer can produce many valid proofs and choose its output. Today this is latent, because proposer selection is deterministic from public data (`pbft.rs:290`) and only tests call `select_validators`. Replace it with RFC 9381 ECVRF before a VRF output steers anything; that is a separate change.
- **libp2p peer identity and transport.** libp2p has no ML-DSA peer IDs. Consensus messages are authenticated at the application layer, so a forged transport identity cannot forge a vote. The data on the wire is public, so harvest-now-decrypt-later does not apply.
- **secp256k1 / Ethereum.** Staking, gas and the relayers depend on Ethereum, which is not post-quantum itself.
- **BLS aggregation** (`consensus/src/bls_aggregate.rs`, cross-chain checkpoint relayers). Not post-quantum, and no standardized post-quantum aggregate exists. Tracked separately.

## 3. Construction

**Message.** For a protocol message `M` (unchanged byte strings: `creg-pbft-v1:<phase>:<hash>`, the vote message, and so on):

```
M' = "creg-hybrid-sig-v1" || 0x00 || M
```

- The Ed25519 component is `Ed25519.Sign(sk_ed, M')`, verified with `verify_strict`.
- The ML-DSA component is `ML-DSA-44.Sign(sk_pq, M', ctx = "creg-hybrid-sig-v1")`: pure ML-DSA (FIPS 204 section 5.2), not HashML-DSA.
- A hybrid signature is valid only if **both** components verify over `M'`.

**Non-separability.** The Ed25519 component signs `M'`, never `M`. Stripping the ML-DSA half therefore leaves a signature that does not verify as a pre-activation Ed25519 signature over `M`. That prevents a downgrade where someone replays half of a hybrid signature against a legacy verifier. Tests must prove this (section 10).

**Encoding.** New optional fields, so encodings before activation stay byte-identical:

- `ValidatorSet` entries gain `pq_pubkey`: 1,312 bytes, lowercase hex, `skip_serializing_if = "Option::is_none"`. `pubkey` stays the Ed25519 key, used by the VRF and by history.
- `BlockSignature` and the other signature structs gain `pq_signature`: 2,420 bytes, lowercase hex, skipped when absent.
- Lengths are checked exactly before any decoding or verification.
- Height rule: below `H`, `pq_signature` must be absent and `signature` is Ed25519 over `M`. At or above `H`, both are required and both are over `M'`. Verifiers therefore take a scheme argument derived from the height; nothing guesses the scheme from field presence alone.

**Key identifiers.** A 1,312-byte key must not be copied into every signed message. Messages that embed a validator public key (the package vote message does) switch to `key_id = sha256("creg-key-id-v1" || 0x00 || ed25519_pk || mldsa44_pk)` under a new domain `creg-vote-v4` (v3 is taken). The ZK slashing public input becomes `key_id`, split into the existing two 128-bit limbs.

## 4. Keys

- **Independent seeds.** A hybrid signing key is two independently generated 32-byte seeds: the Ed25519 seed and the ML-DSA-44 seed `ξ` (FIPS 204 `KeyGen_internal`). The ML-DSA seed must never be derived from the Ed25519 seed. A quantum attacker recovers the Ed25519 private key from the public key; if the ML-DSA seed came from it, the attacker would get both and the hybrid would protect nothing.
- **Wallet-derived publisher keys.** `cli/src/auth.rs::derive_ed25519_key_from_signature` derives a key from an EIP-712 wallet signature. A quantum attacker who recovers the wallet's secp256k1 key (public once the address has sent a transaction) can recompute that RFC 6979-deterministic signature. A wallet-derived ML-DSA key would therefore give no post-quantum protection. Wallet-derived publishers stay Ed25519-only and are labelled classical; hybrid publishers generate their ML-DSA seed independently.
- **Key file v2.** `{"version": 2, "scheme": "hybrid-ed25519-mldsa44-v1", "ed25519_seed": "<64 hex>", "mldsa44_seed": "<64 hex>"}`, written by `creg keygen <role> --scheme hybrid`. Only seeds are stored, never the 2,560-byte expanded key. Seeds are zeroized after use. The CLI prints no derived `0x` address for hybrid keys.
- **Node environment.** `CREG_VALIDATOR_KEY` is unchanged (Ed25519 seed). A new `CREG_VALIDATOR_PQ_KEY` holds the ML-DSA seed. At or above `H`, a validator without it refuses to sign rather than signing Ed25519-only.
- **Binding.** A validator proves possession of its ML-DSA key with a binding statement, `creg-pq-binding-v1|<validator_id>|<ed25519_pk>|<mldsa44_pk>|<chain_id>`, signed by **both** keys. The four bootstrap validators get `pq_pubkey` in the next signed chain spec; validators that join later submit the binding with their registration. The validator-set hash commits to `pq_pubkey` when present.

## 5. Size and performance

| | Ed25519 | ML-DSA-44 | Hybrid |
|---|---|---|---|
| Public key | 32 B | 1,312 B | 1,344 B |
| Signature | 64 B | 2,420 B | 2,484 B (4,968 hex characters) |
| Stored secret | 32 B seed | 32 B seed | 64 B |

The binding constraint is the PBFT view-change certificate. It holds up to `q = 2f+1` votes, each carrying a prepared certificate of up to `q` PREPARE signatures, so up to `q × (q+1)` signatures. The P2P handler rejects messages over 1 MiB (`node/src/p2p.rs:799`).

| Validators | `q` | Hybrid signatures in a full certificate | Hex size |
|---|---|---|---|
| 4 (today) | 3 | 12 | about 60 KB |
| 19 | 13 | 182 | about 0.9 MB |
| 22 | 15 | 240 | about 1.19 MB: over the limit |

So hex-encoded hybrid certificates stop fitting at around 20 validators, well below the default `max_validators: 50` in `common/src/chain_spec.rs`. Decision 4 chooses the fix before activation: deduplicate identical PREPARE signatures inside a certificate, use binary encoding for signature bytes, or raise the limit.

Hedged ML-DSA signing gives a different signature each time the same message is signed. Anything that compares signature bytes rather than signed content must be checked. The ADR-016 equivocation detector is already safe: it compares block hashes, not signatures (`consensus/src/slashing_relay.rs:320`). Gossip deduplication and any cache keyed on signature bytes still need checking in increment 4.

## 6. Implementation and library policy

- **One implementation per language, pinned exactly.** Every node runs the same binary, so a verifier that is too lax does not split the fleet. It does weaken the post-quantum half. A disagreement between the Rust node and the JS tooling, or between two crate versions, does cause one.
- **Library upgrades are consensus changes.** A fix that makes verification stricter (as CVE-2026-24850 did in RustCrypto `ml-dsa`, which had accepted signatures with repeated hint indices) can make a node reject a historical block that an older binary accepted. Before any ML-DSA library upgrade, re-verify every hybrid signature in the chain with the new version, then roll it out through the observer canary like any consensus change.
- **Differential corpus in CI.** NIST ACVP ML-DSA-44 vectors (key generation, signing, verification), plus a malformed-signature set: wrong lengths, repeated or out-of-order hint indices, out-of-range `z`, wrong context string, a component signed over `M` instead of `M'`. Run against the Rust and JS verifiers; any disagreement fails the build.
- **Candidates** (decision 1):
  - RustCrypto `ml-dsa` 0.1.x. Pure Rust; its README says it has never been independently audited; affected by CVE-2026-24850 before 0.1.0-rc.4.
  - `aws-lc-rs`. Already in `Cargo.lock` (1.16.2), backed by the AWS-LC C library; at last check its ML-DSA API was behind an unstable feature. Confirm before choosing.
  - JS tooling needs one implementation (for example `@noble/post-quantum`), held to the same corpus.

## 7. Security analysis

| Concern | Position |
|---|---|
| Classical forgery through an ML-DSA bug | Ed25519 must still verify, so a broken ML-DSA verifier alone does not allow forgery. |
| Quantum forgery after `H` | ML-DSA must still verify. Independent seeds (section 4) keep the quantum attacker from recovering it via the Ed25519 key. |
| Downgrade by stripping one component | Prevented by signing `M'` (section 3) and by the height rule: at or above `H`, Ed25519-only signatures are rejected. |
| Quantum forgery of history before `H` | Pre-activation blocks are Ed25519-only. A quantum attacker could forge an alternative pre-`H` history for a node syncing from genesis (a long-range attack). Mitigation: a hybrid-signed weak-subjectivity checkpoint taken soon after `H`, which syncing nodes must require. |
| Wallet-derived keys | Classical only (section 4). Shown as such in the CLI and the explorer. |
| Verifier divergence | Section 6: one pinned implementation per language, differential corpus, upgrades re-verify history. |
| Denial of service through large messages | Exact length checks before decoding; per-peer rate limits unchanged; certificate size limit (section 5). |
| Secret handling | Seeds only, zeroized after use, same custody as the Ed25519 key. No logging of key material. |

## 8. Rejected alternatives

- **Replace Ed25519 with ML-DSA-44.** Smaller and simpler, but it bets every signature on implementations that are young, unaudited, and had a verification CVE in 2026.
- **ML-DSA-65 or ML-DSA-87.** Larger keys and signatures (ML-DSA-65: 1,952-byte keys, 3,309-byte signatures), which worsen section 5. Category 2 is adequate inside a hybrid. Revisit if guidance changes.
- **SLH-DSA (hash-based).** The most conservative assumption, but signatures of about 8 to 50 KB and slow signing rule it out for consensus. Worth reconsidering for the rarely used chain-spec signing root.
- **FN-DSA (Falcon).** Smaller signatures, but less mature, and its floating-point signing is hard to implement safely.
- **Deriving the ML-DSA seed from the Ed25519 seed.** Gives no quantum protection (section 4).
- **One opaque concatenated signature field.** Changes the encoding of existing structures. Separate optional fields keep encodings before activation byte-identical.

## 9. Decisions needed

| # | Decision | Recommendation |
|---|---|---|
| 1 | ML-DSA library for Rust and JS | Choose after a short evaluation of `aws-lc-rs` (already in the tree) against RustCrypto `ml-dsa`, using the section 6 corpus. Pin exact versions. |
| 2 | Hedged or deterministic ML-DSA signing | Hedged (the FIPS 204 default; better against fault and side-channel attacks), with the section 5 checks. |
| 3 | Publisher grace period after `H` | Long enough for external publishers to re-key; publish the sunset height with the activation. |
| 4 | Certificate-size strategy | Deduplicate PREPARE signatures within certificates; binary encoding if that is not enough. Must be settled before `H`. |
| 5 | Hybrid chain-spec signing key | Yes, from the next signed spec, generated in a recorded ceremony (see the CR-07 procedure). |
| 6 | How a live chain adopts `H` | The same mechanism as the CR-07 activation height (a new signed spec with a coordinated rollout). Settle it once for both. |
| 7 | VRF | Replace it with RFC 9381 ECVRF in a separate change, before any VRF output influences selection. |

## 10. Increments

1. **Scheme abstraction, no behaviour change.** A `common::crypto` module with a `SignatureScheme` type, key and signature types, and scheme-aware sign and verify functions. Move every Ed25519 call site to it. `signed_message_formats.rs` and every other test pass unchanged, and output is byte-identical.
2. **ML-DSA-44 and hybrid behind the abstraction, unused by consensus.** Library choice (decision 1), ACVP and malformed-signature corpus, non-separability tests, key file v2, `creg keygen --scheme hybrid`, JS verifier and the CI differential check.
3. **Key binding.** `pq_pubkey` in the validator set and the chain spec, the binding statement, validator-set hash commitment, and the registration API.
4. **Activation.** `hybrid_signature_activation_height` in `consensus_params` (skipped when unset), height-dependent verification, `creg-vote-v4`, certificate-size fix (decision 4). Then a local four-validator simulation (this needs the four-validator fixture noted as missing in AUD-03), an observer canary that verifies hybrid traffic, and the r7 fleet.
5. **Off-consensus signers.** Chain spec, weak-subjectivity checkpoints, transparency log and witnesses, threshold shares, publisher signatures with the grace period, explorer and sidecar.
6. **Sunset.** Reject Ed25519-only publisher signatures for new publications after the grace period. Historical verification keeps the Ed25519 path permanently.

### 10.1 Increment 1 result (2026-11-21)

- `common/src/crypto.rs` provides `SignatureScheme` (only `Ed25519` today, deliberately not `#[non_exhaustive]`), `PublicKey`, `Signature`, `SigningKey`, `VerifyMode` and `CryptoError`. Each function calls exactly the `ed25519-dalek` routine its call sites used before; `CryptoError` displays the library's own message, so error text is unchanged.
- Every production signing and verification site now goes through it: common (chain spec, ownership, transparency log, validator snapshots), consensus (PBFT, vote accumulator, VRF), node (API, authority adapter, block producer, startup, namespace consensus, P2P, package admission, PBFT safety journal, revocation gossip, sync, validator pipeline), CLI (auth, keygen, multisig, publish, recovery, witness, and both signing examples), resolver, threshold-encryption, validator and zk-validator. Test modules and integration tests still use `ed25519-dalek` directly, as an independent reference.
- **Verification mode is preserved per site.** Strict (`verify_strict`): PBFT phase and certificate signatures, the package-vote accumulator, P2P vote gossip, sync block-signature checks, the PBFT safety journal, validator-set transitions, the light client and the VRF. Permissive (`verify`): chain spec, namespace authority and controller signatures, transparency-log heads and witnesses, the HTTP vote endpoint, block announcements, revocations, publisher signatures, key rotation, validator identity proofs, the authority adapter, threshold shares, AAA proofs and ZK slashing evidence. Tightening a permissive site is a separate, consensus-relevant change. Note one inconsistency to review: votes are checked strictly over gossip but permissively on the HTTP vote endpoint.
- Evidence: the workspace builds and `cargo test --workspace` passes, with no changes to `consensus_golden_vectors` or `signed_message_formats`. Seven new tests cover the RFC 8032 test vector, byte-for-byte agreement with direct `ed25519-dalek` calls in both modes, the strict/permissive split on a small-order key, wrong-length rejection with unchanged messages, and that `Debug` never prints a secret. `cargo clippy --workspace --all-targets -D warnings` and `cargo fmt --check` are clean.

Test requirements across increments: ACVP vectors; the malformed corpus; stripped-component and swapped-component rejection; an Ed25519 component over `M'` must fail as a legacy signature over `M`; height-boundary tests on both sides of `H`; certificate-size tests at the largest supported validator count; and replay of the full pre-`H` chain on the new binary.
