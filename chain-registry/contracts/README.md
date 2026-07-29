# Smart contracts

Solidity sources for Chain Registry L1. Deploy with Foundry (`forge test`, `forge script`).

## Status

| Contract | File | Status |
|----------|------|--------|
| Registry, Staking, Governance, ZK stack, etc. | `*.sol` | **Active** — see root [README.md](../../README.md) contract table |
| CrossChainRegistry | `CrossChainRegistry.sol` | **Planned** — D4 / SEC-303c; spec `cross_chain: false` on Sepolia |
| PrivateRegistry | *(not in tree)* | **Planned** — D5 / SEC-306a; ISSUE-004; no `PrivateRegistry.sol` until enterprise commitment |

```bash
cd chain-registry/contracts
forge test
```

## BatchOperations (`BatchOperations.sol`)

Gas-batched package submissions and reward claims. Intended for publishers who stake CREG and use an **authorized submit relay**.

| Flow | Requirement |
|------|-------------|
| `batchSubmitPackages` | Caller must have `staking.stakedBalance(msg.sender) > 0` |
| Relay path | `BatchOperations` must be allowlisted via `Registry.setPackageSubmitRelay(batchOps, true)` (governance) |
| Publisher identity | Each item is stored under `msg.sender` (the staked publisher) via `Registry.submitPackageFor` |

Unauthorized relays revert with `NotAuthorizedSubmitRelay`. See `contracts/test/Registry.t.sol` (`test_batchSubmitPackages*`).

## Finalize relay model (`Registry.sol`)

`enforceFinalizeRelays` defaults to **true** on deploy. When enabled, only addresses in `packageFinalizeRelays` may call `finalizePackage`. Governance may relax enforcement for permissionless relaying on testnets.

Every `ValidatorSig` is an EIP-712 `PackageVerdict` bound to the chain, Registry address, package key, content hash, `approved` decision, current quorum percentage, exact `Staking.validatorSetEpoch`, and an `issuedAt`/`expiresAt` interval no longer than 15 minutes. Membership changes advance the epoch even when the active count returns to the same value. Finalization requires the signed epoch to equal the current epoch, the signer to remain active, a low-s signature, and the live quorum; `validatorSignedForPackage` and the package key prevent replay. This changes the research-alpha `finalizePackage` tuple ABI and requires contract redeployment plus relayer/signer updates; no production migration is represented by this repository change.
