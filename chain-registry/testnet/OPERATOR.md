# Chain Registry testnet — operator runbook

**Network:** Sepolia (`creg-testnet-1`, L1 chain id `11155111`)  
**Current topology:** the checked-in Sepolia specification has one validator
and no bootnodes. It is a coordinated research alpha and has no Byzantine
tolerance claim. [TESTNET_SCALE_ARCHITECTURE.md](../../docs/TESTNET_SCALE_ARCHITECTURE.md)
is a future design, not evidence of independently operated validators.

This runbook is for **maintainers** operating edge services, optional seed validators, and cohort onboarding. External validators follow [VALIDATOR_ONBOARDING_CHECKLIST.md](../../docs/VALIDATOR_ONBOARDING_CHECKLIST.md).

---

## Topology

| Tier | Where | Purpose |
|------|-------|---------|
| **Edge** | `creg-testnet-vm` (public) | TLS, spec server, IPFS, faucet, waitlist/hub proxies |
| **Observer pool** | GCP MIG (optional) | Public API reads (`api.testnet.cregnet.dev`) |
| **Seed validators** | `creg-validator-vm` (private VPC) | Optional bootstrap quorum — **not** the validator cap |
| **External validators** | Operator-owned hosts | Genesis Alpha cohort — one host per operator |
| **Sepolia RPC** | `creg-sepolia-geth-vm` (private) | `CREG_ETH_RPC` for fleet and observers |

Cohort waves: [GENESIS_ALPHA_COHORT_PLAYBOOK.md](../../docs/GENESIS_ALPHA_COHORT_PLAYBOOK.md).

**Retired:** Sepolia `docker-compose.3node.yml` lab — archived under [archive/3node-lab/](./archive/3node-lab/README.md). Do not use for new work.

---

## Clock synchronization

Every validator and block-producing host must maintain synchronized UTC with
NTP, chrony, or an equivalent monitored time service. Live PBFT proposals use
whole Unix seconds and fail closed when they are more than 60 seconds behind or
ahead of a receiver's clock. Operators must treat drift near that bound as a
liveness incident. Historical sync and rebuild still require each child
timestamp to be strictly later than its parent, but do not compare old blocks
to the current wall clock.

---

## Maintainer seed fleet (optional)

Small maintainer fleet on GCP seeds P2P and early quorum while external operators join.

```powershell
.\testnet\init-sepolia-fleet-env.ps1
# Set CREG_EDGE_INTERNAL_IP, CREG_ETH_RPC (internal Geth) in testnet/sepolia-fleet.env
```

On `creg-validator-vm` (Linux):

```bash
./testnet/start-validator-fleet-gcp.sh
```

Compose: `docker-compose.validator-fleet.yml` + `docker-compose.fleet-sandbox.yml` (nsjail default).

Deploy from workstation:

```powershell
.\testnet\gcp\deploy-validator-fleet.ps1
.\testnet\gcp\verify-fleet-sandbox.ps1
```

Detail: [GCP-VALIDATOR-FLEET.md](../../docs/GCP-VALIDATOR-FLEET.md).

---

## External validator onboarding

Each operator runs **one validator per machine** using `docker-compose.validator.yml` or the release `creg-node` binary.

| Step | Reference |
|------|-----------|
| Cohort invite | [GENESIS_ALPHA_COHORT_PLAYBOOK.md](../../docs/GENESIS_ALPHA_COHORT_PLAYBOOK.md) |
| Full checklist | [VALIDATOR_ONBOARDING_CHECKLIST.md](../../docs/VALIDATOR_ONBOARDING_CHECKLIST.md) |
| Publisher/dev flows | [PUBLIC_TESTNET_QUICKSTART.md](../../docs/PUBLIC_TESTNET_QUICKSTART.md) |
| Alpha limits | [TESTNET_PHASE_SCOPE.md](../../docs/TESTNET_PHASE_SCOPE.md) |

**NET-301 acceptance (scale):** L1 shows ≥2 active staked validators from **independent operators**, `CREG_PBFT_ALLOW_SMALL_CLUSTER_QUORUM` unset on public hosts, publish reaches `verified` with PBFT quorum — not single-validator override.

---

## Sandbox (SANDBOX-301 / MAL-001)

| Profile | Image | `CREG_DEV_SANDBOX` | Host |
|---------|-------|-------------------|------|
| Public validator | `Dockerfile.secure` (nsjail) | `false` | Linux |
| Maintainer seed fleet | `chain-registry-node-secure:fleet` | `false` (forced by fleet-sandbox overlay) | `creg-validator-vm` |
| Local Sepolia bootstrap | `docker-compose.testnet.yml` | Sepolia L1 | Local |

```powershell
.\testnet\build-3node-secure-image.ps1   # builds nsjail secure image (name legacy; used by fleet)
.\testnet\sandbox-301-verify.ps1
```

Public validator profiles must never run `CREG_DEV_SANDBOX=true`.

---

## IPFS pinning (IPFS-001 / IPFS-002)

```powershell
.\testnet\gcp\run-ipfs-pin-check.ps1
.\testnet\gcp\run-ipfs-pin-check.ps1 -CheckOnly
```

Reports: `testnet/ipfs-pin-logs/`. Non-zero exit → Scenario 4 in [INCIDENT_RESPONSE_RUNBOOK.md](../../docs/INCIDENT_RESPONSE_RUNBOOK.md).

---

## Distribution (DIST-301)

```bash
git tag v0.1.2-testnet && git push origin v0.1.2-testnet
```

```powershell
.\testnet\verify-dist-301.ps1 -Version v0.1.2-testnet
```

---

## Sepolia L1 JSON-RPC (public)

| URL | Purpose |
|-----|---------|
| `https://explorer.<base>/rpc` | Sepolia `eth_*` for wallets and `cast` |
| `https://api.<base>/rpc` | **CREG only** — not Sepolia |

Verify: `.\testnet\verify-sepolia-rpc-endpoints.ps1`

---

## L1 anchoring & reorg safety

| Knob | Default | Meaning |
|------|---------|---------|
| `CREG_VALIDATOR_SET_FINALITY_LAG` | `2` (testnet) | Blocks before applying L1 staking events |
| `CREG_BRIDGE_SELF_APPROVE` | `true` | Set `false` for multi-signer governance anchor path |

Anchors: `GET /v1/bridge/anchors`. Reorgs: `GET /v1/reorgs`.

## Validator-set checkpoint rollout

Before enabling certified committee authority, independently verify a signed validator-set transition certificate: its effective height, prior block hash, prior/next validator-set hashes, and distinct `floor(2n/3)+1` prior-set signatures must all match the canonical chain view.

1. Each independent verifier retains its own canonical block observation and prepares the strict anchor format described in [`config/weak-subjectivity/README.md`](../config/weak-subjectivity/README.md). Do not use a block hash copied only from the checkpoint publisher.
2. On an offline verification host, validate and sign an initial evidence record. Supply an independent Ed25519 evidence key; never reuse a validator, node, chain-spec, or deployment key:

   ```powershell
   node .\scripts\verify-weak-subjectivity-checkpoint.mjs publish `
     --checkpoint .\checkpoint.json `
     --previous-validator-set .\previous-validator-set.json `
     --canonical-anchor .\canonical-anchor.json `
     --max-anchor-age-blocks 1000 `
     --verifier-id independent-operator-1 `
     --verified-at 2026-07-29T12:00:00Z `
     --evidence-signing-key X:\offline\checkpoint-evidence-key.pem `
     --initial-release `
     --output .\checkpoint-evidence.operator-1.json
   ```

   The age limit and timestamp above are examples, not network policy. Select and record the approved values for the actual rollout. The command refuses an existing output path.
3. A second host verifies the record against the verifier key obtained out of band:

   ```powershell
   node .\scripts\verify-weak-subjectivity-checkpoint.mjs verify-evidence `
     --evidence .\checkpoint-evidence.operator-1.json `
     --trusted-verifier-public-key REPLACE_WITH_PINNED_LOWERCASE_ED25519_PUBLIC_KEY
   ```

   Repeat independently with the required operator set. Compare the logical transition ID and exact checkpoint environment hash in their signed records. The tool validates supplied evidence; it does not query the chain or declare an operator-supplied anchor canonical.
4. Distribute the exact `checkpoint_env_json` from the approved signed record out of band as `CREG_WEAK_SUBJECTIVITY_CHECKPOINT`. Put that single-line value into the node environment, keep `CREG_REQUIRE_CERTIFIED_COMMITTEES=false`, and restart one observer canary.
5. Compare `GET /v1/validator/set-transitions` on the canary and two independent peers. The checkpoint's logical transition ID and every descendant height/set hash must agree. Exact quorum-witness digests may differ when peers received different valid prior-set quorum subsets; verify every witness independently rather than requiring byte-identical signature lists.
6. Rebuild the canary from an empty node data directory. Verify that it retrieves history only from a peer chain containing the exact configured checkpoint and rejects missing, conflicting, or fork-anchored history.
7. Validate receipt of `creg/v1/validator-set-transitions`; every peer must independently reject replay conflicts, stale height, invalid quorum, and the wrong canonical-tip anchor.
8. After a successful fleet rehearsal, enable `CREG_REQUIRE_CERTIFIED_COMMITTEES=true` on one admission canary, verify certified assignment proofs, and roll it out gradually.

For a later checkpoint, every verifier supplies its own preceding signed record with `--predecessor-evidence` and its separately pinned signer key with `--trusted-predecessor-public-key`. The release gate rejects exact replay, same-height conflict, rollback, and a transition that does not directly descend from the last published validator authority. Publish every validator-set transition; do not skip an authority transition and then treat an unverified gap as lineage.

The checkpoint is a bootstrap trust anchor, not a privileged validator role or an API override. Do not enable mandatory mode without the verified certificate and rehearsal evidence.

### Recovery and rollback

1. Remove a node with checkpoint/authority errors from admission traffic; do not let it choose a newer checkpoint from gossip or a single peer.
2. Preserve its logs, `validator-set-history.json`, configured checkpoint JSON, and canonical tip hash for incident comparison.
3. Compare the configured certificate and descendant history with at least two independently controlled operators. A same-height certificate mismatch or anchor mismatch is a security incident, not an automatic longest-chain decision.
4. Restore the last independently verified checkpoint and rebuild the node. Keep mandatory mode enabled during recovery; disabling it reopens compatibility authority and requires an explicit, documented emergency decision.
5. Re-admit the node only after its canonical block hashes, certificate history, and certified committee assignments match the rehearsed fleet evidence.

Never silently move the weak-subjectivity checkpoint backward. A checkpoint replacement requires the same independent verification and staged rehearsal as the original rollout.

## CR-07 namespace-authority rollout

This rollout is separate from validator-set checkpoint rollout. Do not enable
authoritative namespace ownership merely because the schema or adapter
boundary compiles.

### Preconditions

1. Obtain independently reviewed npm and PyPI self-contained verifier
   executables plus the exact policy and trust-root bundles used to build them.
   The generated Python `creg-pypi-authority-verifier` console launcher is not
   eligible: its hash does not cover `sidecar.py`, the interpreter, or imported
   packages. The repository's locked PyInstaller one-file recipe is a build
   path, not a reviewed release; reproduce and review it separately for every
   target operating system, architecture, and pinned CPython build.
2. Hash every complete executable, policy, and trust-root bundle. Put the
   authority, adapter ID/version, verifier SHA-256, policy SHA-256, trust-root
   SHA-256, and activation height in the chain spec consensus parameters. If a
   verifier is a directory or image rather than one self-contained file, stop:
   the current Rust/config contract cannot pin it and must be extended first.
3. Regenerate the versioned chain spec and genesis identity, sign it, and
   distribute it through the existing pinned spec-signing key path. Activation
   or policy values supplied only through
   `CREG_NAMESPACE_AUTHORITY_ACTIVATION_HEIGHT` or
   `CREG_NAMESPACE_AUTHORITY_POLICIES` are not authorized.
4. Mount each verifier read-only at its absolute path and configure:

   | Variable | Initial rollout guidance |
   |----------|--------------------------|
   | `CREG_NAMESPACE_AUTHORITY_NPM_VERIFIER_PATH` | Empty until the reviewed npm verifier is mounted |
   | `CREG_NAMESPACE_AUTHORITY_PYPI_VERIFIER_PATH` | Empty until a reviewed self-contained PyPI artifact—not the generated Python console launcher—is mounted |
   | `CREG_NAMESPACE_AUTHORITY_VERIFIER_TIMEOUT_SECS` | Start at `30`; never exceed the protocol cap |
   | `CREG_NAMESPACE_AUTHORITY_MAX_EVIDENCE_BYTES` | Start at `2097152` |
   | `CREG_NAMESPACE_AUTHORITY_MAX_ARTIFACT_BYTES` | Start at `16777216`; hard maximum `67108864` |
   | `CREG_NAMESPACE_AUTHORITY_MAX_RESULT_BYTES` | Start at `65536` |
   | `CREG_NAMESPACE_AUTHORITY_MAX_CONCURRENT_VERIFICATIONS` | Start at `2`; hard maximum `4` |
   | `CREG_NAMESPACE_AUTHORITY_ARCHIVE_MAX_BYTES` | Shared evidence/artifact quota; default `10737418240` (10 GiB) |
   | `CREG_NAMESPACE_AUTHORITY_ARCHIVE_MAX_ENTRIES` | Shared file-count quota; default `100000` |
   | `CREG_NAMESPACE_AUTHORITY_ARCHIVE_MIN_RETENTION_SECS` | Minimum pressure-GC age; default `31536000` (365 days) |

   PyPI v2 hex-encodes both binary inputs. At the protocol hard maxima, the
   artifact and evidence therefore occupy 144 MiB before JSON metadata; the
   sidecar reserves a 145 MiB stdin ceiling. If limits are raised above the
   defaults, size process memory for the canonical request, decoded bytes,
   archive parser, and Sigstore verifier rather than treating the framing
   ceiling as a safe memory budget.

5. Run each verifier under a dedicated unprivileged identity and reviewed OS
   sandbox with outbound network denied, explicit CPU/memory/process limits,
   and read-only executable/policy/trust mounts. Hash pinning and an
   environment-cleared subprocess are not an OS sandbox. The deployment must
   also prevent executable replacement between hash verification and launch.
   Before enabling the PyPI one-file artifact, the node must create a fresh
   absolute private per-invocation directory below its data-dir scratch root,
   export only the adapter protocol plus `TEMP`, `TMP`, and `TMPDIR` pointing
   there, hold the directory through child exit, and remove it afterward. Use
   mode `0700` or an equivalent service-only ACL; never preserve ambient temp
   variables or grant write access to the verifier/policy/trust directory.
6. Confirm that exact accepted evidence is stored below `CREG_DATA_DIR` under
   content-addressed references of the form
   `creg-evidence:sha256:<digest>`. The separately fetched official artifact
   must be stored at
   `authority-artifacts/<sha256|sha512>/<prefix>/<digest>.bin`, using SHA-512
   for npm and SHA-256 for PyPI. Bearer tokens must never enter either archive,
   logs, or a certificate. The node makes one locked quota decision for the
   evidence/artifact pair, rejects corrupt existing entries, and rolls back
   entries newly written by a failed pair operation. Under byte or entry-count
   pressure it removes the oldest entries only after the configured minimum
   retention age; the pair currently being attested is protected from that
   collection. Crash-left `.tmp` entries become eligible after 24 hours.
   Configure capacity alerts and an independently reviewed backup/legal
   retention policy before accepting public traffic: this local pressure GC is
   not an archival-compliance or disaster-recovery system. The mutation lock
   is process-local; never point two node processes at the same
   `CREG_DATA_DIR`.
7. Generate a distinct high-entropy `CREG_OPERATOR_API_KEY` for each operator
   trust domain. Both authority routes require it through `X-Operator-Key` or
   `Authorization: Bearer`; a checked-in sample/default key is not acceptable.
   Keep it server-side: never pass it through a `VITE_*` build argument,
   JavaScript bundle, public proxy configuration, URL, or request body.
   `CREG_OPERATOR_PUBKEY` is public verification material and cannot unlock
   private routes; do not configure it as an API-key substitute.
   Rotate it independently of controller and validator signing keys.

### Canary procedure

1. Before activation, create a controller-signed intent for a disposable npm
   or PyPI namespace and include it in a new upstream release. The intent binds
   chain, genesis, purpose, namespace, controllers, nonce, issuance, and
   expiry, but not its enclosing artifact digest.
2. Submit official `evidence_url` and `artifact_url` values independently to
   canary validators using the authenticated
   `POST /v1/validator/namespace-authority/attest` route. Each validator must
   separately fetch bounded evidence and artifact bytes, compute SHA-256 for
   evidence and native SHA-512 (npm) or SHA-256 (PyPI) for the artifact, and
   run its own pinned verifier; copying another validator's result is not
   verification.
   Include `X-Operator-Key: <CREG_OPERATOR_API_KEY>` or the equivalent bearer
   header. The controller-signed challenge is still required; the service key
   does not authorize a namespace.
3. Assemble matching signed statements into a certificate, submit it to the
   `POST /v1/validator/namespace-authority/certificates` validation
   route, and require distinct signatures from `floor(2n/3) + 1` members of
   the authenticated anchor set.
4. Attach the completed certificate to the final v4 authorization. Verify that
   changing the package, certificate, controller, chain/genesis, validity
   interval, policy pin, or anchor causes rejection.
5. Repeat for recovery and verify that recovery without a fresh recovery-
   purpose certificate fails. Rebuild and restart the canary, then exercise a
   fork/reorg whose abandoned branch contained an authority certificate.
6. Only after the real sidecars, multi-operator results, evidence retention,
   and rollback drill have been independently reviewed should operators sign a
   spec with a future activation height.

For npm, require a registry-origin signal that the release actually used
trusted-publisher authentication mode. Provenance availability alone is not
that signal and must not be promoted into namespace authority. The exact npm
trust material, authentication-mode policy, and verifier artifact are external
rollout inputs that still need independent review.

The attestation endpoint permits 2 requests per source IP per minute and at
most 4 concurrent verifier processes per node (default 2). A `429` or busy
response is a capacity signal, not permission to use a different proof path.
Sidecar `verified_at` is checked only for validator-local freshness and is
excluded from the shared proof/statement; otherwise independently timed
validators could not sign identical statement bytes.

### Legacy CLI compatibility

Legacy CLI signing and `creg multisig` are disabled by default when no complete
v4 identity is configured. A pre-activation development network must set
`CREG_ALLOW_LEGACY_PUBLISHER_PROOF=true` explicitly, and the CLI still refuses
the legacy path when `CREG_PRODUCTION=true`. Never enable this flag on the
public fleet or after CR-07 activation.

Both endpoints are implemented in the node router and appear in the shipped
OpenAPI document. Do not schedule activation until their adversarial results
are recorded and the deployed canary binary is confirmed to expose and protect
them; repository implementation alone is not deployment evidence.

### Failure and rollback

1. Remove an affected validator from authority-attestation traffic. Leave
   deterministic certificate validation enabled; disabling it would reopen
   namespace takeover paths.
2. Preserve the controller intent, official evidence/artifact URLs, both
   content-addressed archive entries, native artifact digest, verifier
   executable digest, policy/trust-root digests, npm authentication-mode signal
   where applicable, signed statement, certificate, canonical anchor, and logs.
3. Stop issuing new statements by clearing the affected verifier path or
   removing the service route, then rotate `CREG_OPERATOR_API_KEY` if exposure
   is possible. Never replace a verifier binary in place while retaining its
   old digest.
4. Compare the certificate and anchor with at least two independently
   controlled validators. A statement mismatch, evidence mismatch, or
   same-height anchor conflict is a security incident.
5. If a reorg abandons the certificate anchor, discard that certificate and
   repeat attestation against the new canonical anchor. Do not replay an
   abandoned-branch certificate.
6. Correct consensus pins only through a newly reviewed, coordinated, signed
   chain-spec rollout. An environment override is not a rollback.

There is no claim yet that production npm/PyPI sidecars, a real signed
activation spec, independent security review, or operator rehearsal exist.
All four remain explicit testnet gates.

---

## Routine operations

| Task | Command |
|------|---------|
| Resume public hosting | `.\testnet\gcp\resume-testnet-hosting.ps1` |
| Start seed fleet (VM) | `./testnet/start-validator-fleet-gcp.sh` |
| Hosting verify | `.\testnet\hosting-301-verify.ps1 -BaseDomain testnet.cregnet.dev` |
| Stop hosting (cost) | `.\testnet\gcp\stop-testnet-hosting.ps1` |
| Local CI | `cargo test --workspace`; `cd contracts && forge test` |

---

## Public hosting (HOSTING-301)

Runbook: [gcp/README.md](./gcp/README.md)

```powershell
.\testnet\prepare-public-hosting.ps1 -BaseDomain testnet.YOUR_DOMAIN -AcmeEmail you@example.com
.\testnet\hosting-301-verify.ps1 -BaseDomain testnet.YOUR_DOMAIN
```

---

## References

- [TESTNET_SCALE_ARCHITECTURE.md](../../docs/TESTNET_SCALE_ARCHITECTURE.md)
- [TESTNET_SEPOLIA_RUNBOOK.md](../../docs/TESTNET_SEPOLIA_RUNBOOK.md)
- [WHAT_TO_DO_NEXT.md](../../docs/WHAT_TO_DO_NEXT.md)
- [GCP-VALIDATOR-FLEET.md](../../docs/GCP-VALIDATOR-FLEET.md)
