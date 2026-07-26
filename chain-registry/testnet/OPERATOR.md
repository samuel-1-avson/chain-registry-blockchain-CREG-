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

1. Distribute that exact verified certificate out of band as `CREG_WEAK_SUBJECTIVITY_CHECKPOINT`.
2. Put the single-line JSON value into the node's environment file, keep `CREG_REQUIRE_CERTIFIED_COMMITTEES=false`, and restart one observer canary.
3. Compare `GET /v1/validator/set-transitions` on the canary and two independent peers. The exact checkpoint certificate hash and every descendant height/hash must agree.
4. Rebuild the canary from an empty node data directory. Verify that it retrieves history only from a peer chain containing the exact configured checkpoint and rejects missing, conflicting, or fork-anchored history.
5. Validate receipt of `creg/v1/validator-set-transitions`; every peer must independently reject replay conflicts, stale height, invalid quorum, and the wrong canonical-tip anchor.
6. After a successful fleet rehearsal, enable `CREG_REQUIRE_CERTIFIED_COMMITTEES=true` on one admission canary, verify certified assignment proofs, and roll it out gradually.

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

1. Obtain independently reviewed npm and PyPI verifier executables plus the
   exact policy and trust-root bundles used to build them.
2. Hash every executable, policy, and trust-root bundle. Put the authority,
   adapter ID/version, verifier SHA-256, policy SHA-256, trust-root SHA-256,
   and activation height in the chain spec consensus parameters.
3. Regenerate the versioned chain spec and genesis identity, sign it, and
   distribute it through the existing pinned spec-signing key path. Activation
   or policy values supplied only through
   `CREG_NAMESPACE_AUTHORITY_ACTIVATION_HEIGHT` or
   `CREG_NAMESPACE_AUTHORITY_POLICIES` are not authorized.
4. Mount each verifier read-only at its absolute path and configure:

   | Variable | Initial rollout guidance |
   |----------|--------------------------|
   | `CREG_NAMESPACE_AUTHORITY_NPM_VERIFIER_PATH` | Empty until the reviewed npm verifier is mounted |
   | `CREG_NAMESPACE_AUTHORITY_PYPI_VERIFIER_PATH` | Empty until the reviewed PyPI verifier is mounted |
   | `CREG_NAMESPACE_AUTHORITY_VERIFIER_TIMEOUT_SECS` | Start at `30`; never exceed the protocol cap |
   | `CREG_NAMESPACE_AUTHORITY_MAX_EVIDENCE_BYTES` | Start at `2097152` |
   | `CREG_NAMESPACE_AUTHORITY_MAX_ARTIFACT_BYTES` | Start at `16777216`; hard maximum `67108864` |
   | `CREG_NAMESPACE_AUTHORITY_MAX_RESULT_BYTES` | Start at `65536` |
   | `CREG_NAMESPACE_AUTHORITY_MAX_CONCURRENT_VERIFICATIONS` | Start at `2`; hard maximum `4` |

5. Confirm that exact accepted evidence is stored below `CREG_DATA_DIR` under
   content-addressed references of the form
   `creg-evidence:sha256:<digest>`. The separately fetched official artifact
   must be stored at
   `authority-artifacts/<sha256|sha512>/<prefix>/<digest>.bin`, using SHA-512
   for npm and SHA-256 for PyPI. Bearer tokens must never enter either archive,
   logs, or a certificate.
6. Generate a distinct high-entropy `CREG_OPERATOR_API_KEY` for each operator
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
   canary validators using the planned authenticated
   `POST /v1/validator/namespace-authority/attest` route. Each validator must
   separately fetch bounded evidence and artifact bytes, compute SHA-256 for
   evidence and native SHA-512 (npm) or SHA-256 (PyPI) for the artifact, and
   run its own pinned verifier; copying another validator's result is not
   verification.
   Include `X-Operator-Key: <CREG_OPERATOR_API_KEY>` or the equivalent bearer
   header. The controller-signed challenge is still required; the service key
   does not authorize a namespace.
3. Assemble matching signed statements into a certificate, submit it to the
   planned `POST /v1/validator/namespace-authority/certificates` validation
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

The two endpoint names above are rollout placeholders until they appear in the
shipped OpenAPI document and their adversarial tests are recorded. Do not
schedule activation against a binary that does not expose and protect them.

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
