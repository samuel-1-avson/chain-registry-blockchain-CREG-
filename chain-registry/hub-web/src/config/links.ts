export const EXTERNAL_LINKS = {
  explorer:
    import.meta.env.VITE_EXPLORER_URL ??
    "https://explorer.testnet.cregnet.dev",
  faucet:
    import.meta.env.VITE_FAUCET_URL ?? "https://faucet.testnet.cregnet.dev",
  waitlist:
    import.meta.env.VITE_WAITLIST_URL ?? "https://waitlist.cregnet.dev",
  spec:
    import.meta.env.VITE_SPEC_URL ??
    "https://spec.testnet.cregnet.dev/chain-spec.json",
  docs:
    import.meta.env.VITE_DOCS_URL ??
    "https://github.com/samuel-1-avson/chain-registry-blockchain-CREG-/blob/main/docs/PUBLIC_TESTNET_QUICKSTART.md",
  api: import.meta.env.VITE_API_URL ?? "https://api.testnet.cregnet.dev",
  apiDocs:
    import.meta.env.VITE_API_DOCS_URL ??
    `${import.meta.env.VITE_API_URL ?? "https://api.testnet.cregnet.dev"}/api-docs/`,
} as const;

export const SEPOLIA_CHAIN_ID = 11_155_111;
