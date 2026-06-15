# Genesis Alpha cohort playbook (Wave 1)

**Updated:** 2026-06-13  
**Audience:** Operators inviting the first coordinated testnet cohort  
**Canonical repo:** [chain-registry-blockchain-CREG-](https://github.com/samuel-1-avson/chain-registry-blockchain-CREG-)  
**Release tag:** `v0.1.2-testnet`

This playbook turns waitlist **Genesis Alpha** registrants (positions 1ΓÇô10,000) into active publishers or validators on the public Sepolia testnet fleet. **Vanguard Beta** (10,001+) uses the same technical steps but ships in a later wave after Alpha stabilizes.

---

## Cohort model

| Tier | Codename | Waitlist slots | Wave | When to invite |
|------|----------|----------------|------|----------------|
| **alpha** | Genesis Alpha | 1ΓÇô10,000 | Wave 1 | Now (coordinated batches) |
| **beta** | Vanguard Beta | 10,001+ | Wave 2 | After Wave 1 soak + SEC-401 booking |

Tier assignment is automatic at registration time (`position <= 10_000` ΓåÆ alpha). See waitlist `TIER_DEFINITIONS` in [Creg-waitlist](https://github.com/samuel-1-avson/Creg-waitlist).

**Do not** promise mainnet tokens, guaranteed NFTs, or production security. Frame everything as **public alpha** on Sepolia.

---

## Wave 1 batch sizing

| Batch | Size | Purpose |
|-------|------|---------|
| Pilot | 5ΓÇô10 wallets | Smoke full path (install ΓåÆ faucet ΓåÆ stake ΓåÆ publish or observe) |
| Batch A | 25ΓÇô50 | Publishers only ΓÇö `creg publish` + explorer verify |
| Batch B | 10ΓÇô20 | Validators only ΓÇö stake + node compose (operator-reviewed) |
| Batch C | 100+ | After pilot + A/B logs clean for 48h |

Track each invite in a simple sheet: `wallet`, `tier`, `path` (publish | validate | observe), `invite_sent`, `cli_installed`, `staked`, `first_action_at`, `blockers`.

Template: [genesis-alpha-wave1-tracker.csv](./genesis-alpha-wave1-tracker.csv) ΓÇö refresh with `.\testnet\gcp\export-genesis-alpha-wave1.ps1 -WriteCsv`

---

## Prerequisites (operator)

Before sending invites:

- [ ] Public fleet healthy ΓÇö [hosting-301-verify.ps1](../chain-registry/testnet/hosting-301-verify.ps1) against `testnet.cregnet.dev`
- [ ] Faucet funded (tCREG + optional native Sepolia ETH drips)
- [ ] [PUBLIC_TESTNET_QUICKSTART.md](./PUBLIC_TESTNET_QUICKSTART.md) reviewed (contract addresses current)
- [ ] [TESTNET_PHASE_SCOPE.md](./TESTNET_PHASE_SCOPE.md) linked in every invite (verified semantics, alpha limits)
- [ ] SEC-401 outreach sent or vendor booked ([SEC-401-VENDOR-OUTREACH.md](./SEC-401-VENDOR-OUTREACH.md))

---

## Invite email template (Genesis Alpha)

**Subject:** CREG Genesis Alpha ΓÇö your Wave 1 testnet invite

**Body:**

```
Hi {{name}},

You're in the CREG Genesis Alpha cohort (waitlist {{position}}).

CREG is a supply-chain registry for chain artifacts ΓÇö signed publishes, IPFS pins, and validator verification on Sepolia. This is public alpha, not mainnet.

Your path: {{path}}  (publish | validate | observe)

1) Join hub (quests + status)
   https://testnet.cregnet.dev
   Connect wallet ΓåÆ Sign in with Ethereum (Sepolia)

2) Install CLI (v0.1.2-testnet)
   export CREG_GITHUB_REPO=samuel-1-avson/chain-registry-blockchain-CREG-
   ./scripts/install-creg.sh --version v0.1.2-testnet
   Or download from:
   https://github.com/samuel-1-avson/chain-registry-blockchain-CREG-/releases/tag/v0.1.2-testnet

3) Quickstart (stake, publish, or validate)
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

Customize `{{path}}` per registrant role from waitlist metadata when available.

---

## Participant checklist (publish path)

| Step | Action | Verify |
|------|--------|--------|
| 1 | Hub SIWE sign-in | Dashboard shows signed-in state |
| 2 | Install `creg` `v0.1.2-testnet` | `creg --version` |
| 3 | `creg keygen publisher` | Ed25519 key file exists |
| 4 | Fund Sepolia EOA + faucet | Balance on Sepolia |
| 5 | `creg stake --role publisher` (1 tCREG min) | Tx on Sepolia staking contract |
| 6 | IPFS up (local or operator gateway) | `ipfs id` or `CREG_IPFS_URL` |
| 7 | `creg publish` against public API | Package `verified` in explorer |
| 8 | Hub quest: mark publish complete | `/quests` on hub |

Contract defaults: [PUBLIC_TESTNET_QUICKSTART.md ┬º Before you start](./PUBLIC_TESTNET_QUICKSTART.md).

---

## Participant checklist (validate path)

| Step | Action | Verify |
|------|--------|--------|
| 1 | Hub SIWE sign-in | Dashboard |
| 2 | Install CLI + Foundry `cast` | `cast --version` |
| 3 | Fund EOA (100 tCREG min stake) | Faucet + stake tx |
| 4 | Read [OPERATOR.md](../chain-registry/testnet/OPERATOR.md) | Understand topology |
| 5 | Operator provisions validator slot OR self-serve stake | L1 registry shows identity |
| 6 | Run node (compose / operator fleet) | `/v1/health` + peer count |
| 7 | Publish smoke observed as validator | Quorum attestation in logs |

Validator onboarding remains **operator-gated** in Wave 1 ΓÇö enrollment intent via hub only; no private keys in browser.

---

## Observe-only path

For wallets that only want to explore:

1. Hub + explorer links (no stake required)
2. [explorer.testnet.cregnet.dev](https://explorer.testnet.cregnet.dev)
3. Optional: run read-only node with public spec URL

---

## Support macros (Discord / email)

| Symptom | Response |
|---------|----------|
| Wrong network | Switch wallet to Sepolia (11155111); hub shows network guard |
| Faucet cooldown | Wait `cooldown_seconds` from faucet stats; no bypass in alpha |
| `creg publish` rejected | Confirm stake, `CREG_NODE_URL`, IPFS reachable, Ed25519 vs EOA keys |
| SIWE fails | Clock skew, wrong domain, or expired nonce ΓÇö sign in again from hub |
| "Verified" confusion | Link [TESTNET_PHASE_SCOPE.md](./TESTNET_PHASE_SCOPE.md) ΓÇö verified on queried node chain |

---

## Metrics (Wave 1 success)

Track weekly:

- Invites sent / accepted (any hub sign-in)
- CLI installs (self-reported or quest completion)
- Unique publisher addresses with ΓëÑ1 verified package
- Validator candidates ΓåÆ active validators
- Mean time from invite ΓåÆ first on-chain action
- Faucet abuse flags / support volume

**Exit criteria for Wave 2 (Beta):** Pilot + Batch A/B complete, no P0 fleet incidents for 7 days, SEC-401 vendor scheduled.

---

## Related docs

| Doc | Use |
|-----|-----|
| [PUBLIC_TESTNET_QUICKSTART.md](./PUBLIC_TESTNET_QUICKSTART.md) | Technical steps |
| [TESTNET-HUB-DESIGN.md](./TESTNET-HUB-DESIGN.md) | Hub quests & SIWE |
| [NEXT_WORK.md](./NEXT_WORK.md) | P0/P1 readiness checklist |
| [FRIEND_ONBOARDING.md](../chain-registry/testnet/FRIEND_ONBOARDING.md) | Private multi-host alpha (Tailscale) |

---

_Update batch sizes and exit criteria as fleet capacity changes._
