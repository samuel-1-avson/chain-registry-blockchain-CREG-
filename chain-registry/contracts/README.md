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

`enforceFinalizeRelays` defaults to **true** on deploy. When enabled, only addresses in `packageFinalizeRelays` may call `finalizePackage`. Governance may relax enforcement for permissionless relaying on testnets. Per-validator signature replay is blocked via `validatorSignedForPackage`.
