# Frequently asked questions

## What chain is this?

**Ethereum Sepolia** (chain id `11155111`). CREG testnet anchors registry state on Sepolia; your wallet must be on Sepolia for on-chain actions.

## How do I get Sepolia ETH?

Use the [testnet faucet](https://faucet.testnet.cregnet.dev). Limits and eligibility are enforced by the faucet service, not the join hub.

## Do I need a wallet to use this site?

No. Guides on `/`, `/publish`, `/validate`, `/compare`, and `/faq` work without signing in. Connect a wallet when you are ready for transactions or when Phase 2 adds SIWE-protected dashboards.

## Is this the explorer or faucet?

No. This is the **join portal** — onboarding and journey copy. The [explorer](https://explorer.testnet.cregnet.dev) shows blocks and packages; the [faucet](https://faucet.testnet.cregnet.dev) dispenses test funds.

## WalletConnect on mobile

Set `VITE_WALLETCONNECT_PROJECT_ID` in `hub-web/.env` (see `.env.example`). Browser extension wallets work without WalletConnect.

## Where is the source?

This hub is open source in the [chain-registry](https://github.com/samuel-1-avson/chain-registry-blockchain-CREG-) repository under `hub-web/`, `hub-api/`, and `hub/content/`.

## Something broke — who do I contact?

Check repository issues and testnet operator logs. For RPC or API outages, static guides on this site remain available in degraded mode.
