# Real explorer pipeline repair evidence

## Latest continuation (2026-09-26, h6)

The earlier h4/mixed-image/h5 snapshots are historical. All eight actual
candidates now run R18 and share h1–h6 with independently verified COMMIT
certificates. Real h6 web and CLI submission/vote/committed events and the CLI
proof passed. The actual h7 revocation exposed missing signed-intent propagation
and timed out; observer pending retirement also needs deployment. The combined
R20 repair is compiling. See `../2026-09-26-phase-c-events-r16/` for current
success and preserved failure evidence. Public routing remains legacy h0;
cutover is pending.

All HTTP, CLI and browser records in this directory use real running nodes.
Browser checks do not replace API responses or submit synthetic transactions.

## Chain identities

- Public explorer/API: `creg-testnet-1`, h0, genesis
  `068364cf778ede5a0b1c814c59b0314f9a8598a959fa5a8e08ea78bc1b296b53`.
- Separate Phase C fleet: `creg-phase-c-testnet-2026-09-26-r7`, h4, genesis
  `0f2a3b109318f0979807f3d921f72775b2a37ce4eb28415a62d9f53b3766eed4`.
- The signed spec's genesis identity is distinct from the actual block-zero
  hash. Neither public routing nor the default spec was replaced in this run.

## Records

- `before/`, `after/`, `after-local-node/`: actual endpoint payloads and status
  codes. Protected operator endpoints remain HTTP 401 to public callers.
- `candidate-browser-r14/`: 17 real candidate routes on the Phase C r7 node.
- `extended-public-r14/`: eight real public routes on the deployed R14 web
  explorer, with no JavaScript errors or failed requests.
- `api-r13-verified.json`: earlier 22-route public API verification.
- `extended-before/`: address, publisher, search, wallet, governance and about
  pages captured during the wider review. Navigation cancellations appear as
  `net::ERR_ABORTED`; they are retained in the request log.
- `cli-*.txt`, `cli-install-r15.json`, `cli-tui-r15/`: actual R15 executable
  output and redacted console exports. Both installed Windows CLI paths have
  SHA-256 `de45f5811ccc062efb09040f7bcf7d898c336c9d3ff893e5c5ab072e122ef831`.
  The CLI reads the real public chain and candidate chain. A packaged installer
  is still pending.
- `cli-linux-r11.txt`: retained failure evidence. The image's CLI retained an
  earlier label despite the reviewed source containing its replacement.
- `protected-history-r11-verified.txt`: authorized request made inside the
  running container without exporting its operator credential. HTTP 501
  explicitly reports that a durable metrics history service is unconfigured.
- `address-before-r13.json`: retained failure evidence for the EVM address
  proposal counter (0 while validator-1 actually proposed h4).
- `fleet-before-node-update.json`, `fleet-after-node-update.json`: all eight
  real nodes scanner-ready and sharing h4. **V3 still has one retained work
  item; readiness does not establish successful recovery.**
- `fleet-final-r15.json`: fresh read-only snapshot at 2026-09-26 13:32 UTC.
  All eight are online/scanner-ready and share h4. V3 still reports one pending
  block transaction, one durable finalized-work item and one forced-inclusion
  entry. V1/V2/V4 have dead-letter depths 2/1/1; inspect their retained reasons.
  Fleet recovery is not accepted.
- `fleet-capture-ps51-incompatible-r15.json`: preserved first diagnostic from
  invoking the PowerShell 7 capture script under PowerShell 5.1. Its JSON parse
  error is a tooling incompatibility, not node health evidence. The capture
  script now rejects PowerShell versions below 7 before writing evidence.
- `local-node-update.json`: real V1 update, preserving its existing data and
  signing journals. Earlier stopped container is retained for rollback.
- `web-release/`, `deploy-web-release.sh`: first production bundle and guarded
  rollout. The previous web container and compose configuration were retained.
- `release-status.json`: current deployment summary; consult the final status
  fields before treating a build or rollout as complete.
- The public R14 image is `chain-registry-web-explorer:pipelines-r14-20260926`
  (`sha256:1e9b0b54d4bd66b83c3bea3452db371005b4d3bd835bf5571fa27e81824dd244`).
  It remains connected to the legacy public chain at h0; the r7 candidate is
  separate and has not been cut over.

## Limits

The existing real package is revoked. Its current install proof is unavailable
by design; this run did not publish another package to produce an event or
proof. SSE transport and named subscriptions were checked, while delivery of a
future real publication still needs capture. Wallet/relay/staking/faucet writes
were not exercised. The bridge remains disabled by the existing safety freeze.

Durable queue/restart, prepared/COMMIT/view-change and quorum-loss acceptance,
broader scanner coverage, real rewards and coordinated public cutover remain
separate gates. See [remaining work](../../EXPLORER-PIPELINE-REPAIR.md).
