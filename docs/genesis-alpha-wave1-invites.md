# Genesis Alpha Wave 1 ΓÇö pilot invites (operator draft)

**Updated:** 2026-06-13  
**Use with:** [GENESIS_ALPHA_COHORT_PLAYBOOK.md](./GENESIS_ALPHA_COHORT_PLAYBOOK.md), [genesis-alpha-wave1-tracker.csv](./genesis-alpha-wave1-tracker.csv)

---

## Pick wallets (Firebase)

1. Open [Firebase Console](https://console.firebase.google.com/) ΓåÆ project `gen-lang-client-0098858574` ΓåÆ Firestore database `ai-studio-6b167dc8-a078-4526-a86b-de2a8722a753`.
2. Collection: `registrations`.
3. Filter: `tier == alpha`, sort by `position` ascending.
4. Select **5ΓÇô10** diverse roles (mix Publisher / Validator Node / Security Audits if possible).
5. Copy `walletAddress`, `position`, `role` into the tracker CSV.

**Path mapping:**

| Waitlist role | Pilot path |
|---------------|------------|
| Publisher | `publish` |
| Validator Node | `validate` (operator-gated in Wave 1) |
| Security Audits | `observe` |

---

## Pilot roster (Wave 1 ΓÇö exported 2026-06-14)

Firestore has **wallet + role only** (no email). Reach pilots via your own contact channel, then paste the body below.

**Send-ready emails:** From `chain-registry/`, run `.\testnet\prepare-genesis-wave1-invites.ps1` to generate [genesis-alpha-wave1-ready.md](./genesis-alpha-wave1-ready.md) (wallet, position, path filled). Send from your ops mailbox; mark `invite_sent` in the tracker CSV.

Refresh roster: `.\testnet\gcp\export-genesis-alpha-wave1.ps1 -WriteCsv`

| # | Wallet | Position | Path | Status |
|---|--------|----------|------|--------|
| 1 | `0xf4c0bdbb681a61aa0b123e82c04b0d692f53d58e` | 1 | validate | invite_sent: _pending_ |
| 2 | `0x01a20c2882eac884c3957f062bee051247b03d42` | 4 | validate | invite_sent: _pending_ |
| 3 | `0x7820C3F9D272Dc2647Ec944FA92b91D0d7e1F2dc` | 6 | validate | invite_sent: _pending_ |
| 4 | `0xddd8f31dcfddbaaba7b7ac07032f8d714b64683b` | 7 | **publish** | invite_sent: _pending_ |
| 5 | `0xe32a0274655f6bab3396df3ee9639a265750b420` | 8 | validate | invite_sent: _pending_ |

Skipped: `0xVerify985583` (invalid address). Positions 2ΓÇô3 are duplicate wallet `0xf4c0ΓÇª`.

---

## Ready-to-send emails

Replace `{{name}}`, `{{position}}`, `{{wallet}}`, `{{path}}`. Send from your ops mailbox; do not CC private keys.

### Pilot 1 ΓÇö Publisher path

**Subject:** CREG Genesis Alpha ΓÇö your Wave 1 testnet invite

```
Hi {{name}},

You're in the CREG Genesis Alpha cohort (waitlist {{position}}).

CREG is a supply-chain registry for chain artifacts ΓÇö signed publishes, IPFS pins, and validator verification on Sepolia. This is public alpha, not mainnet.

Your path: publish

1) Join hub (quests + status)
   https://testnet.cregnet.dev
   Connect wallet ΓåÆ Sign in with Ethereum (Sepolia)

2) Install CLI (v0.1.2-testnet)
   export CREG_GITHUB_REPO=samuel-1-avson/chain-registry-blockchain-CREG-
   ./scripts/install-creg.sh --version v0.1.2-testnet
   Or download from:
   https://github.com/samuel-1-avson/chain-registry-blockchain-CREG-/releases/tag/v0.1.2-testnet

3) Quickstart (stake, publish)
   https://github.com/samuel-1-avson/chain-registry-blockchain-CREG-/blob/main/docs/PUBLIC_TESTNET_QUICKSTART.md

4) Faucet (Sepolia ETH + tCREG)
   https://faucet.testnet.cregnet.dev?address={{wallet}}

5) Public API
   export CREG_NODE_URL=https://api.testnet.cregnet.dev

Read first (limits & "verified" meaning):
https://github.com/samuel-1-avson/chain-registry-blockchain-CREG-/blob/main/docs/TESTNET_PHASE_SCOPE.md

Reply in this thread if blocked >24h. Do not share private keys or validator keys in email.

ΓÇö CREG testnet ops
```

### Pilot ΓÇö Observe path

Same template; set `Your path: observe` and omit stake/publish emphasis. Point to explorer: https://explorer.testnet.cregnet.dev

### Pilot ΓÇö Validate path

Same template; set `Your path: validate`. Add: "Validator slots are operator-provisioned in Wave 1 ΓÇö reply with your intended stake address and we'll confirm enrollment after hub SIWE."

---

## After send

1. Set `invite_sent` date in [genesis-alpha-wave1-tracker.csv](./genesis-alpha-wave1-tracker.csv).
2. Watch hub sign-ins (`hub_signed_in`) and support volume.
3. Log first on-chain action in `first_action_at` when a pilot publishes or stakes.
