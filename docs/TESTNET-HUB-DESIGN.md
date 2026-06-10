# Testnet Hub -- Join portal design

> **Status:** Design approved (brainstorming lock 2026-06-10)  
> **Updated:** 2026-06-10  
> **URL (target):** `https://join.testnet.cregnet.dev`  
> **Goal:** Dedicated onboarding and growth surface for the CREG Sepolia testnet (`creg-testnet-1`).

Production path for public testnet participation. Explorer, faucet, and operator runbooks remain separate; the hub orients newcomers and tracks guided journeys.

**Related:** [PUBLIC_TESTNET_QUICKSTART.md](./PUBLIC_TESTNET_QUICKSTART.md) | [TESTNET_PHASE_SCOPE.md](./TESTNET_PHASE_SCOPE.md) | [GCP-VALIDATOR-FLEET.md](./GCP-VALIDATOR-FLEET.md)

---

## Understanding summary

- **What:** A dedicated **testnet hub** (new site, not an explorer extension) that explains CREG and guides visitors into **Publish** or **Validate** with equal weight.
- **Why:** Explorer and faucet are operator/utility tools; the project needs a **growth + onboarding surface** that explains contribution paths and nudges participation.
- **Who:** Curious newcomers first, then developers (publish) and operators (validate).
- **Core v1 features:**
  - Public marketing and education (no wallet required)
  - Wallet connect + **SIWE (EIP-4361) for protected actions** (quests, progress, enrollment intent)
  - Light **dashboard** (address, active path, next steps, chain hints)
  - **Guided journeys** (checklists; copy versioned in repo)
  - **On-chain status** where readable (Sepolia balance, validator registration, published packages)
  - **Phased rewards:** v1 = faucet routing + checklist progress; v2 = chain-verified milestones
- **Content:** Hybrid -- core journeys and quest definitions in repo; announcements and lighter marketing copy elsewhere.
- **Non-goals (v1):** Not replacing explorer; not the waitlist product; not a full quest economy; not custom L1 token logic beyond existing test ETH / faucet flows; no validator private keys in browser or hub-api.

---

## Assumptions and NFRs

| Area | Assumption |
|------|------------|
| **Traffic** | Low-moderate testnet traffic (hundreds of MAU, not millions) |
| **Availability** | Same tier as testnet edge (~best-effort); hub downtime does not stop the chain |
| **Security** | SIWE sessions, rate limits, no validator keys in browser; hub never holds mainnet funds |
| **Privacy** | Store wallet address + quest progress only; minimal PII |
| **Maintenance** | Small team; prefer same deploy path as faucet / explorer / Caddy |
| **Chain** | Sepolia (chain id `11155111`); reads via `SEPOLIA_RPC_URL` / internal Geth path per [GCP-SEPOLIA-GETH-INTERNAL.md](./GCP-SEPOLIA-GETH-INTERNAL.md) |
| **CREG API** | Public reads from `https://api.testnet.cregnet.dev` where endpoints exist |
| **Session** | ~24h SIWE-backed session; HTTP-only cookie; nonce single-use |
| **Mobile** | WalletConnect required in wallet stack (mobile browser support) |
| **DB v1** | SQLite on edge VM; Postgres only if multi-instance is required later |

---

## Decision log

| # | Topic | Choice | Rationale |
|---|-------|--------|-----------|
| 1 | Primary audience | Curious visitors + dual paths (Publish and Validate) | Funnel for newcomers; equal weight on both contribution modes |
| 2 | Product shape | **New dedicated site** (not explorer extension) | Explorer stays technical; hub is "start here" |
| 3 | Post-login scope | Dashboard + guided journeys + on-chain status | Full hub experience, not a static brochure |
| 4 | Rewards model | **Phased** -- v1 checklist + faucet routing; v2 on-chain verification | Ship fast; verify milestones on-chain later |
| 5 | Wallet auth | **SIWE for protected actions; public browse without wallet** | Best-practice balance (EIP-4361) |
| 6 | Content ownership | **Hybrid** -- journeys/quests in repo; announcements elsewhere | Engineers own flows; marketing iterates copy off-repo |
| 7 | Backend hosting v1 | **Edge stack** (`hub-web` + `hub-api` on `creg-testnet-vm`) | Same Docker/Caddy ops as faucet/explorer; supports SIWE and quest state |
| 8 | Backend alternative | Cloud Run for `hub-api` only if edge VM is resource-constrained | Defer unless edge is crowded |
| 9 | Hub pattern | **Approach 1 -- Join portal** | Clear boundary vs explorer; deep links to existing tools |
| 10 | Subdomain | **`join.testnet.cregnet.dev`** | Distinct from `api.` / `explorer.` / `faucet.` |
| 11 | Information architecture | Confirmed public + authenticated routes (see below) | User-approved IA |
| 12 | v1 quests | Repo-defined YAML/JSON + server-side progress; chain verify in v2 | Version quests with deploy |
| 13 | DB v1 | SQLite on edge | Simple ops; migrate to Postgres if needed |
| 14 | Quest conflict resolution | Last-write-wins v1 | Acceptable for testnet |
| 15 | Session expiry | ~24h; re-sign on 401 | Standard SIWE session pattern |
| 16 | Faucet integration | Hub routes and eligibility flags; faucet service unchanged | No hub-held drip keys in v1 |
| 17 | Validator enrollment | Intent capture + links to operator docs/scripts in v1; full wizard deferred | Security: never collect private keys |
| 18 | Wallet stack alignment | wagmi + viem + SIWE; share patterns with explorer where practical | Consistent UX; separate hub session |
| 19 | Error posture | Plain language; degraded mode when RPC/API down | Static guides always available |
| 20 | Testing | API unit/integration + Playwright critical path + manual Sepolia walkthrough | Before public announce |

---

## Approach chosen

### Join portal at `join.testnet.cregnet.dev`

Standalone SPA + small API on the existing testnet **edge** stack (`creg-testnet-vm`). Single brand: **Join the testnet**.

```
Internet
   |
   v
join.testnet.cregnet.dev  (Caddy TLS on creg-testnet-vm)
   |
   +-- /*           --> hub-web (static SPA, nginx or Caddy file_server)
   +-- /api/*       --> hub-api (:hub-api port, e.g. 8095)
   |
   |  Deep links (not rebuilt in hub):
   +-- explorer.testnet.cregnet.dev  (blocks, publish UI, validators)
   +-- faucet.testnet.cregnet.dev    (tCREG / Sepolia ETH drip)
   +-- api.testnet.cregnet.dev       (CREG node API reads)
   +-- docs / OPERATOR.md / PUBLIC_TESTNET_QUICKSTART.md
```

| Approach considered | Verdict |
|---------------------|---------|
| Explorer onboarding routes (`/join`, `/quests`) | Rejected for v1 -- mixes learn vs operate UX |
| Docs-first static hub | Too thin for dashboard + SIWE + quests |
| **Join portal (chosen)** | Matches confirmed scope and ops model |

---

## Information architecture and routes

### Public (no wallet)

| Route | Purpose |
|-------|---------|
| `/` | What is CREG testnet? Two cards: **Publish packages** \| **Run a validator** |
| `/publish` | Why publish, prerequisites, links to CLI and [PUBLIC_TESTNET_QUICKSTART.md](./PUBLIC_TESTNET_QUICKSTART.md) |
| `/validate` | Why validate, stake overview, hardware expectations |
| `/compare` | Side-by-side: publisher vs validator vs observer |
| `/faq` | Sepolia ETH, faucet limits, support links |

### Authenticated (SIWE session)

| Route | Purpose |
|-------|---------|
| `/dashboard` | Address, Sepolia balance hint, active path, quest progress, chain hints |
| `/publish/start` | Publish checklist: wallet, faucet, CLI, first package, explorer link |
| `/validate/start` | Validate checklist: wallet, faucet, stake guide, register, run node doc |
| `/quests` | All checklist items and completion state |
| `/status` | On-chain: validator registered? packages published? (read-only aggregation) |

### Global chrome

- Connect wallet (preview without SIWE)
- Sign in (SIWE) when saving progress
- Links: Explorer, Faucet, Docs, Network status pill (from `GET /api/health` + optional CREG API health)

### Handoffs (external)

| Target | URL / artifact |
|--------|----------------|
| Faucet | `https://faucet.testnet.cregnet.dev` (address pre-fill if faucet supports query param) |
| Explorer | `https://explorer.testnet.cregnet.dev` |
| CREG API | `https://api.testnet.cregnet.dev` |
| Operator runbook | `chain-registry/testnet/OPERATOR.md` |
| Friend onboarding | `chain-registry/FRIEND_ONBOARDING.md` (link or render subset) |

---

## Components

| Component | Responsibility | Notes |
|-----------|----------------|-------|
| **hub-web** | React/Vite SPA | wagmi, viem, SIWE client; no secrets |
| **hub-api** | Nonce issue, SIWE verify, sessions, quest CRUD, eligibility flags, chain status aggregation | Rust (Axum) or Node (Hono/Fastify) -- pick one stack in Phase 0 |
| **hub-db** | SQLite v1 | Tables: `sessions`, `quest_progress`, optional `enrollment_intent` |
| **Caddy** | TLS + vhost `CREG_PUBLIC_JOIN_HOST` | `reverse_proxy` to hub-web and hub-api |
| **Chain reader** | Inside hub-api | Sepolia JSON-RPC + optional `api.testnet.cregnet.dev` reads |

### Repo layout (target)

```
chain-registry/
  hub-web/          # Vite SPA
  hub-api/          # SIWE + quests + status
  hub/content/      # quests/*.yaml, journey copy (MDX or markdown)
  testnet/
    docker-compose.hub.yml
    caddy/          # CREG_PUBLIC_JOIN_HOST block added to fleet/hybrid Caddyfiles
```

### Caddy integration

Add to `testnet/caddy/Caddyfile.fleet` (and siblings) after waitlist block:

```
{$CREG_PUBLIC_JOIN_HOST} {
    handle /api/* {
        reverse_proxy 127.0.0.1:{$CREG_HUB_API_PORT}
    }
    handle {
        reverse_proxy 127.0.0.1:{$CREG_HUB_WEB_PORT}
    }
}
```

Env vars (add to `sepolia-3node.env.example` in Phase 4):

```env
CREG_PUBLIC_JOIN_HOST=join.testnet.cregnet.dev
CREG_HUB_WEB_PORT=8094
CREG_HUB_API_PORT=8095
```

DNS: A record `join.testnet.cregnet.dev` -> edge VM public IP (`35.225.225.20` today).

### Docker compose

New overlay `testnet/docker-compose.hub.yml` stacked with `docker-compose.cloud-edge.yml` or `docker-compose.3node-services.yml`:

| Service | Image | Port (host) |
|---------|-------|-------------|
| `hub-web` | nginx serving `hub-web/dist` or dev server | `8094` |
| `hub-api` | built from `hub-api/` | `8095` |
| `hub-db` | volume mount for SQLite file | internal |

**hub-web** does not hold secrets. **hub-api** does not hold validator private keys or faucet drip keys.

---

## Data flows

### 1. Public browse

```
Browser --> hub-web (static) --> render MDX/markdown from hub/content
```

No backend required. Network status pill may call `GET /api/health` (optional anonymous).

### 2. SIWE sign-in

```
hub-web                    hub-api                    hub-db
   | GET /api/auth/nonce  -->  store nonce (TTL)  -->
   | <-- nonce + message  |
   | wallet.signMessage   |
   | POST /api/auth/verify --> verify EIP-4361, create session -->
   | <-- Set-Cookie (HttpOnly, Secure, SameSite=Lax)
```

- Nonce single-use; short TTL (e.g. 5 minutes).
- Session ~24h; quest writes require valid session.
- Address stored lowercase-normalized; display checksummed in UI.

### 3. Quest progress

```
hub-web (SIWE cookie) --> PATCH /api/quests/:id --> hub-db (address, quest_id, state, updated_at)
hub-web                 --> GET  /api/quests      --> merge with hub/content/quests/*.yaml definitions
```

States: `locked` | `available` | `in_progress` | `completed`.

### 4. Chain status

```
hub-api --> Sepolia RPC (balance, staking contract reads)
        --> api.testnet.cregnet.dev (validator set, package status if exposed)
        --> aggregate GET /api/status?address=0x...
```

Cached briefly (e.g. 30-60s) to limit RPC load. Degraded response when upstreams fail.

### 5. Faucet handoff

```
Quest step "claim Sepolia ETH" completed (manual confirm v1)
  --> hub sets faucet_eligible flag (server-side, SIWE-gated)
  --> UI CTA links to faucet.testnet.cregnet.dev
  --> v2: faucet webhook marks step complete automatically
```

Hub does **not** replace faucet crypto in v1.

---

## v1 quest model and v2 hooks

### Quest definitions (repo)

Versioned under `hub/content/quests/` as YAML or JSON. Example structure:

```yaml
id: publish_first_package
path: publish
title: Publish your first package
order: 4
verification: manual   # v2: chain | api
prerequisites: [siwe_signin, install_cli]
```

Deploy loads definitions into hub-api at startup (or hub-web bundles public metadata).

### Shared quests (both paths)

| Quest id | v1 behavior |
|----------|-------------|
| `connect_wallet` | Client-detected; optional progress without SIWE |
| `siwe_signin` | Completed on successful `/api/auth/verify` |
| `claim_sepolia_eth` | Manual confirm or link to faucet; v2 webhook |

### Publish path

| Quest id | v1 behavior | v2 hook |
|----------|-------------|---------|
| `read_publish_guide` | Manual complete | -- |
| `install_cli` | Manual complete | Detect `creg --version` self-report optional |
| `publish_first_package` | Manual complete | Verify via CREG API / chain event |

### Validate path

| Quest id | v1 behavior | v2 hook |
|----------|-------------|---------|
| `read_operator_guide` | Manual complete | -- |
| `fund_wallet` | Manual + balance hint from RPC | Auto when balance > threshold |
| `register_validator` | Manual + explorer link | Detect staking contract registration |
| `run_node` | Honor system + link to OPERATOR.md / compose | Optional enrollment intent queue for fleet ops |

### Abuse controls (v1)

- Rate-limit quest writes per address and per IP.
- SIWE required for all progress mutations.
- Faucet docs stress testnet-only; hub never sends tokens directly in v1.

### v2 hooks (document now, build in Phase 5)

- Indexer or cron: verify `PackagePublished`, validator registry events --> auto-complete quests.
- Webhook from faucet on successful drip --> complete `claim_sepolia_eth`.
- Optional: hub-enrolled validator waitlist for fleet operators.
- On-chain quest verification replaces `verification: manual` entries.

---

## Wallet auth

### Pattern: connect for browse, SIWE for actions

| Mode | Wallet | SIWE | Capabilities |
|------|--------|------|--------------|
| **Public** | None | No | All marketing routes, FAQ, compare |
| **Connected preview** | Connect only | No | Show address on dashboard preview; read-only balance hint |
| **Signed in** | Connected | Yes | Save quest progress, enrollment intent, eligibility flags |

### Implementation notes

- Stack: **wagmi + viem + SIWE** (align libraries with explorer where practical).
- **WalletConnect** required for mobile browsers.
- Wrong network: banner "Switch to Sepolia"; block chain-dependent quest completion until chain id `11155111`.
- Never collect or transmit private keys.
- Hub maintains its **own session** (separate from explorer) because quest state is server-side.

### hub-api auth endpoints (v1)

| Method | Path | Auth |
|--------|------|------|
| GET | `/api/health` | Public |
| GET | `/api/auth/nonce` | Public (rate-limited) |
| POST | `/api/auth/verify` | Public (body: SIWE message + signature) |
| POST | `/api/auth/logout` | Session |
| GET | `/api/quests` | Session |
| PATCH | `/api/quests/:id` | Session |
| GET | `/api/status` | Session (or public with address query -- prefer session-bound) |

---

## Error handling and edge cases

| Scenario | Behavior |
|----------|----------|
| SIWE fails / expired session | "Sign in again"; public pages work; quest writes return `401` |
| Wrong network (not Sepolia) | Banner + switch network CTA; block chain-dependent steps |
| RPC / CREG API down | `/status` shows degraded; quests still save; chain steps show "unavailable" |
| Faucet rate-limited | Link to faucet; explain cooldown; do not auto-complete quest |
| hub-api down | SPA loads static content; dashboard shows offline state |
| hub-db lost/corrupt | Quest progress reset acceptable for testnet; sessions re-created on SIWE |

### Edge cases

| Case | v1 handling |
|------|-------------|
| Same address, multiple browsers | Last-write-wins on quest state |
| User pursues both paths | Dashboard shows Publish and Validate tracks |
| Already a validator | `/status` hints + explorer link; v2 auto-completes validate quests |
| No Sepolia ETH | Dashboard CTA to faucet first |
| Mobile wallets | WalletConnect in connect flow |
| Security | Enrollment = links + intent only; no key upload |

Errors are **plain language** in UI; server logs include request id, no stack traces to clients.

---

## Testing strategy

| Layer | Tests |
|-------|--------|
| **hub-api** | SIWE verify unit tests (valid/invalid/expired nonce); quest state integration tests; rate-limit smoke |
| **hub-web** | Component tests for quest list; network banner |
| **E2E** | Playwright: land --> connect --> SIWE --> complete one manual quest --> dashboard reflects state |
| **Deploy** | `GET /api/health` in compose healthcheck; staging on same compose stack as faucet |
| **Manual** | One full Publish path + one full Validate path on Sepolia before announce |
| **Security** | Session cookie flags; CSRF on mutating routes; no secrets in hub-web bundle |

Optional CI job: `hub-api` tests on PR touching `hub-api/`; Playwright on PR touching `hub-web/`.

---

## Implementation phases

### Phase 0 -- Scaffold (no user-facing features)

**Goal:** Empty hub runs locally and behind Caddy stub.

| Task | Detail |
|------|--------|
| Create `hub-web/` | Vite + React + TypeScript; placeholder `/` route; health banner stub |
| Create `hub-api/` | Minimal server with `GET /api/health`; Dockerfile |
| Create `hub/content/` | `.gitkeep` + sample `quests/README.md` |
| Add `testnet/docker-compose.hub.yml` | `hub-web`, `hub-api`, SQLite volume |
| Caddy stub | `CREG_PUBLIC_JOIN_HOST` block in `Caddyfile.fleet` / `Caddyfile.hybrid` / `Caddyfile.with-faucet` |
| Env example | Document `CREG_PUBLIC_JOIN_HOST`, `CREG_HUB_WEB_PORT`, `CREG_HUB_API_PORT` in `sepolia-3node.env.example` |
| Local script | `testnet/start-hub-local.ps1` or document compose one-liner |
| Docs pointer | Entry in `DELIVERABLES_INDEX.md` |

**Exit criteria:** `curl http://localhost:8095/api/health` OK; hub-web serves on `8094`; compose up/down documented.

---

### Phase 1 -- Public pages + wallet connect

| Task | Detail |
|------|--------|
| Implement public routes | `/`, `/publish`, `/validate`, `/compare`, `/faq` |
| Content | Markdown/MDX from `hub/content/` for journey copy |
| Global chrome | Header links to explorer, faucet, docs |
| wagmi + WalletConnect | Connect wallet; show address in header |
| Network guard | Sepolia chain id check + banner |
| Styling | Share design tokens with explorer where practical (fonts, dark theme) |

**Exit criteria:** All public routes render without backend; wallet connects on desktop and mobile.

---

### Phase 2 -- SIWE + quests API + dashboard

| Task | Detail |
|------|--------|
| hub-api auth | Nonce, verify, session cookie, logout |
| hub-db schema | `sessions`, `quest_progress` migrations |
| Quest loader | Load YAML definitions at startup |
| Quest API | `GET /api/quests`, `PATCH /api/quests/:id` (session-gated) |
| hub-web SIWE | Sign-in flow; session persistence |
| Routes | `/dashboard`, `/quests` |
| Rate limiting | Per IP and per address on auth and quest writes |

**Exit criteria:** User can SIWE sign-in, see quests, mark a manual step complete, refresh and see persisted state.

---

### Phase 3 -- Publish/validate checklists + status reads

| Task | Detail |
|------|--------|
| Quest content | Full shared + publish + validate quest YAML |
| Routes | `/publish/start`, `/validate/start`, `/status` |
| Chain reader | Sepolia balance; staking/registry reads; CREG API package/validator hints |
| `GET /api/status` | Aggregate endpoint with degraded mode |
| Faucet handoff | `faucet_eligible` flag + CTA linking to faucet |
| Enrollment intent | Optional `POST /api/enrollment/intent` (validate path, SIWE-gated) |

**Exit criteria:** End-to-end checklist UX for both paths; status page shows live or degraded chain data.

---

### Phase 4 -- GCP deploy integration

| Task | Detail |
|------|--------|
| Stack integration | Add hub overlay to `docker-compose.cloud-edge.yml` or `docker-compose.3node-services.yml` |
| DNS | A record `join.testnet.cregnet.dev` -> edge VM |
| Env on VM | Set `CREG_PUBLIC_JOIN_HOST` via `push-env.ps1` / `hosting.env` |
| Deploy script | `testnet/gcp/deploy-hub.ps1` (mirror `deploy-waitlist.ps1` pattern) |
| Verify script | Extend `hosting-301-verify.ps1` or add `hub-verify.ps1` for join vhost + `/api/health` |
| Chain spec | Optional link in `chain-spec.sepolia.json` services block |

**Exit criteria:** `https://join.testnet.cregnet.dev` serves SPA; `https://join.testnet.cregnet.dev/api/health` returns OK from production edge.

---

### Phase 5 (v2) -- On-chain verification and faucet webhooks

| Task | Detail |
|------|--------|
| Quest verification engine | `verification: chain | api` in quest YAML |
| Event indexer / cron | Auto-complete publish and validate milestones |
| Faucet webhook | `POST /api/webhooks/faucet` (shared secret) marks drip quest complete |
| Auto-complete UX | Dashboard badges when chain detects registration or publish |
| Postgres migration | If SQLite becomes a bottleneck |
| Fleet enrollment queue | Optional operator view for validate intents |

**Exit criteria:** At least one publish and one validate quest auto-completes from chain/API evidence without manual click.

---

## Related docs

| Doc | Topic |
|-----|--------|
| [PUBLIC_TESTNET_QUICKSTART.md](./PUBLIC_TESTNET_QUICKSTART.md) | Publisher / validator CLI flows |
| [TESTNET_PHASE_SCOPE.md](./TESTNET_PHASE_SCOPE.md) | Alpha limits and verified semantics |
| [GCP-VALIDATOR-FLEET.md](./GCP-VALIDATOR-FLEET.md) | Edge + validator fleet layout |
| [GCP-RPC-ARCHITECTURE.md](./GCP-RPC-ARCHITECTURE.md) | API/RPC ingress |
| [WAITLIST_FIREBASE_DEPLOY.md](./WAITLIST_FIREBASE_DEPLOY.md) | Separate waitlist product (not hub) |
| [../chain-registry/testnet/OPERATOR.md](../chain-registry/testnet/OPERATOR.md) | Validator operator runbook |
| [../chain-registry/DELIVERABLES_INDEX.md](../chain-registry/DELIVERABLES_INDEX.md) | Scripts and compose index |
