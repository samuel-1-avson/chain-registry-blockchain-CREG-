# Contract Fixes & Redeploy Plan

The CREG Solidity contracts are **immutable** (no proxies/UUPS). The fixes
below live in source and only take effect on a **new deployment**. This
document records the changes and the migration plan.

## Fixes in this change

### 1. `Registry._recoverSigner` — EIP-2 low-s malleability (Medium)
`ecrecover` accepts both `s` and `n - s`, so a third party could derive a
second valid signature for the same `finalizePackage` payload. `Staking`
already rejected high-s; `Registry` did not.

- **Fix:** normalize `v` and reject high-s (`s > secp256k1n/2`), returning
  `address(0)` on a malleable signature — which callers already treat as a
  non-active / invalid signer. Mirrors `Staking._recoverSigner`.
- **File:** `contracts/Registry.sol` `_recoverSigner`.
- **Risk:** none for honest signers (canonical low-s signatures are unaffected).

### 2. PackageInsurance → Staking slash ACL mismatch (High, latent)
`PackageInsurance.resolveClaim` calls `staking.slash(...)`, but `Staking.slash`
/ `slashSeverity` only permitted `registry` or `governance`. An approved
insurance claim would have reverted. (PackageInsurance is **not deployed** —
`feature_flags.insurance = false` — so this is latent.)

- **Fix:** `Staking` gains a governance-managed slasher allowlist
  (`authorizedSlashers` + `setSlasher(address,bool)` + `SlasherUpdated`). The
  `slash`/`slashSeverity` checks now also accept authorized slashers. Empty by
  default — nothing can slash unless governance explicitly authorizes it.
- **Files:** `contracts/Staking.sol`. `PackageInsurance.sol` is unchanged; its
  call now succeeds **only after** governance authorizes its address.
- **Tests:** `contracts/test/SlashPriority.t.sol` `AuthorizedSlasherTest`
  (run with `forge test` — not validated locally; no `forge` toolchain on the
  dev box used for this change).

## Why a redeploy is required

`Staking` is immutable and **holds real staked CREG**; `Registry` is immutable
and references `staking` as an immutable dependency. Patching either means a
fresh deployment of the contract suite and re-pointing everything at the new
addresses. There is no in-place upgrade path.

## Migration plan (bundle into the next authority deployment)

These fixes are **not** an emergency:
- The low-s issue has no active exploit path on the current single-operator
  testnet (finalize is relay-allowlisted; recovered non-signers are rejected).
- The insurance ACL is latent (insurance not deployed).

Bundle them into the next planned redeploy via
`testnet/deploy-sepolia-new-authority.ps1`.

### Steps

1. **Announce + freeze:** pause new staking/publishing; note that the testnet
   stake ledger resets (or coordinate withdrawals — see step 2).
2. **Drain old stake (optional):** on the current testnet, validators/publishers
   `initiateUnbonding` → `withdrawValidatorStake` and withdraw publisher stake
   from the **old** `Staking`. For a low-value alpha it is acceptable to abandon
   the old ledger and re-stake on the new contracts instead.
3. **Deploy the new suite:** `deploy-sepolia-new-authority.ps1` deploys
   `CregToken` (or reuse the existing token), `Staking` (with fixes), `Registry`
   (with fix), `Governance`, `Reputation`, `ZKVerifier`, `Appeal`,
   `ValidatorRewards`, `VRF`, wiring `staking.setContracts(registry, reputation)`
   and `cregToken.transferOwnership(governance)`.
   - Set **`GOVERNANCE_THRESHOLD >= 2`** with independent `GENESIS_SIGNERS` (see
     the warning now emitted by `deploy-sepolia.ps1`). Then set
     `CREG_BRIDGE_SELF_APPROVE=false` on the bridge.
4. **(If enabling insurance later):** deploy `PackageInsurance`, then have
   governance call `staking.setSlasher(<packageInsurance>, true)`. Without this
   step, approved claims revert by design.
5. **Update the chain spec:** write the new contract addresses into
   `testnet/chain-spec.sepolia.json` (and the spec-server copy), then
   **re-sign and re-publish** the spec + `.sig` via `finalize-sepolia-spec.ps1`
   / `sync-sepolia-spec-server.ps1`. (The unbonding fix from the prior commit
   re-signs in the same pass.)
6. **Update node/operator env:** `CREG_TOKEN_ADDR`, `CREG_STAKING_ADDR`,
   `CREG_REGISTRY_ADDR`, `CREG_GOVERNANCE_ADDR`, faucet/relayer token addresses.
   `deploy-sepolia-new-authority.ps1` already rewrites the manifest/env it owns.
7. **Re-seed the genesis validator** (1000 CREG) and re-register validator
   identities (now gossiped fleet-wide automatically — one POST suffices).
8. **Verify:** `verify-sepolia-rpc-endpoints.ps1`, `l2-gate-verify.ps1 -Live`,
   and confirm `active_validators >= 2` with PBFT progress before reopening.

### Rollback
If the new deployment misbehaves, point node env + chain spec back at the
previous contract addresses and re-sign the spec. Keep the old addresses in the
deployment manifest until the new suite has soaked.

## Verification checklist (post-redeploy)
- `forge test` passes including `AuthorizedSlasherTest` and the existing
  `SlashPriority` / `ConsensusAdmission` / `Registry` suites.
- A high-s `finalizePackage` signature is rejected (no double-finalize).
- With insurance enabled, an approved claim slashes the publisher only after
  `setSlasher(insurance, true)`; before authorization it reverts.
- Governance threshold is `>= 2`; bridge runs with `CREG_BRIDGE_SELF_APPROVE=false`.
