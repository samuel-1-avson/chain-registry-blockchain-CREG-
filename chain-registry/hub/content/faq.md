# Frequently asked questions

## What chain is the testnet on?

CREG testnet operations use **Ethereum Sepolia** (chain id `11155111`) for staking, registry, and token contracts. The hub will prompt you to switch networks when your wallet is on another chain.

## How do I get Sepolia ETH?

Use the testnet [faucet](https://faucet.testnet.cregnet.dev). Rate limits and cooldowns apply — the faucet service is separate from this hub. If you are rate-limited, wait for the cooldown or use a public Sepolia faucet for raw ETH.

## Do I need a wallet to use this site?

No. All marketing and guide pages work without a wallet. Connect a wallet to preview your address in the header; **sign-in with SIWE** (saved progress) arrives in Phase 2.

## Is this mainnet? Can I lose real money?

No. This is a **testnet lab**. Use throwaway keys and small amounts only. Never reuse mainnet seed phrases.

## Where is the block explorer?

[explorer.testnet.cregnet.dev](https://explorer.testnet.cregnet.dev) — blocks, packages, validators, wallet tools, and publisher UI.

## Where is the API?

[api.testnet.cregnet.dev](https://api.testnet.cregnet.dev) — node HTTP API and RPC ingress per testnet docs.

## WalletConnect does not appear

Mobile and some desktop browsers need WalletConnect. Set `VITE_WALLETCONNECT_PROJECT_ID` in `hub-web/.env` (free project at [WalletConnect Cloud](https://cloud.walletconnect.com)). Local dev still works with injected browsers (MetaMask, Rabby) without a project id.

## Something is broken — where do I report it?

Open an issue on the [chain-registry GitHub](https://github.com/samuel-1-avson/chain-registry-blockchain-CREG-/issues) with steps, wallet type (if relevant), and screenshots. For validator emergencies, follow your operator runbook first.

## What comes next on this hub?

- **Phase 2** — SIWE sign-in, quest API, dashboard.
- **Phase 3** — Publish/validate checklists, on-chain status reads.
- **Phase 4** — Production deploy at `join.testnet.cregnet.dev`.
