# Hub API remediation status

Tracks implementation of [WEB_APPS_DATABASE_API_AUDIT.md](./WEB_APPS_DATABASE_API_AUDIT.md) items (2026-06-12).

| # | Item | Status | Notes |
|---|------|--------|-------|
| 1 | SQLite schema + migrations | **Done** | `hub-api/migrations/001_initial.sql`; `better-sqlite3`; parameterized queries only |
| 2 | Session middleware (HttpOnly, Secure, SameSite=Lax) | **Done** | `hub-api/src/middleware/session.ts`; SIWE verify creates session |
| 3 | Rate-limit `/api/status/public` and `/api/auth/*` | **Done** | In-memory limiter; env-tunable windows |
| 4 | Remove `VITE_OPERATOR_API_KEY` from production explorer build | **Done** | Removed from `docker-compose.cloud-edge.yml`; local compose retains dev key |
| 5 | Fix hub health metadata | **Done** | `phase: "1"`, `db: ready\|not_configured\|error` |
| 6 | Faucet PoW / edge rate limits | **Partial** | Cloud-edge default `FAUCET_POW_DISABLED=false`; Cloud Armor still blocked if quota=0 |
| 7 | Faucet key in Vault | **Documented** | See `testnet/gcp/faucet-vault-env.example` |
| 8 | Firebase waitlist audit | **Documented** | [WAITLIST_FIREBASE_AUDIT_CHECKLIST.md](./WAITLIST_FIREBASE_AUDIT_CHECKLIST.md) |
| 9 | Hub-api + explorer smoke tests | **Done** | `testnet/hub-explorer-smoke.ps1` wired into `public-alpha-rehearsal.ps1 -Execute` |

---

## Implemented files

| Path | Change |
|------|--------|
| `chain-registry/hub-api/` | DB, migrations, session + rate-limit middleware, SIWE auth routes |
| `chain-registry/hub-web/src/api/status.ts` | Public probes no longer expect upstream URLs |
| `chain-registry/testnet/docker-compose.cloud-edge.yml` | No operator key in explorer build; PoW default false |
| `chain-registry/testnet/hub-explorer-smoke.ps1` | Live smoke checks |
| `chain-registry/testnet/public-alpha-rehearsal.ps1` | Runs smoke script when `-Execute` |

---

## Blocked / manual ops

### Cloud Armor (WAA-004 companion)

- `GCP_ARMOR_POLICY_NAME` in `hosting.env.example` — policy creation may fail when project Cloud Armor quota is 0.
- **Manual:** Request quota increase, attach policy to edge backend, or keep faucet PoW enabled (`FAUCET_POW_DISABLED=false`).

### Cloud Run hub-api persistence

- `deploy-hub-api-cloudrun.ps1` may still set `HUB_DB_PATH=/tmp/hub.db` (ephemeral).
- **Manual:** Use Cloud SQL or mount persistent volume before relying on quest/session state in Cloud Run.

### Firebase waitlist

- Rules and App Check live in the waitlist Firebase project — use [WAITLIST_FIREBASE_AUDIT_CHECKLIST.md](./WAITLIST_FIREBASE_AUDIT_CHECKLIST.md).

### Faucet Vault migration

1. Store key at Vault path `secret/data/creg/faucet` (field `private_key`).
2. On edge VM set `CREG_SECRETS_BACKEND=vault`, `VAULT_ADDR`, `VAULT_TOKEN` (or AppRole).
3. Remove `FAUCET_PRIVATE_KEY` from plain env files.
4. Redeploy faucet container; verify `GET /health` and a test drip on Sepolia.

See `testnet/gcp/faucet-vault-env.example`.

---

## Ops deploy (2026-06-12)

| Step | Status | Notes |
|------|--------|-------|
| Cloud Run hub-api (SQLite image) | **Done** | Revision `creg-hub-api-00002-7tw`; health shows `db: ready`, `migrationsApplied: 1` |
| VM repo sync | **Done** | `sync-local-repo.ps1` → `creg-testnet-vm` |
| Cloud-edge redeploy (explorer w/o operator key) | **Done** | `start-cloud-edge-gcp.sh` completed; `hub-explorer-smoke.ps1` PASS |
| `FAUCET_POW_DISABLED=false` on edge | **Done** | Set in VM `sepolia-3node.env` |
| Cloud Armor | **Blocked** | `SECURITY_POLICIES` quota limit **0** — keep PoW on until quota granted |
| Faucet Vault | **Pending** | `FAUCET_PRIVATE_KEY` still in env; no Vault on edge — see `faucet-vault-env.example` |
| Firebase waitlist audit | **Pending** | Human checklist — `WAITLIST_FIREBASE_AUDIT_CHECKLIST.md` |

Cloud Run URLs (both serve new revision):

- `https://creg-hub-api-108687509435.us-central1.run.app`
- `https://creg-hub-api-wmkf4sobla-uc.a.run.app` (legacy alias in `sepolia-3node.env`)

**Cloud Run DB:** `HUB_DB_PATH=/tmp/hub.db` remains ephemeral — SIWE sessions reset on cold start until Cloud SQL or volume mount.

---

## Verification

```powershell
cd chain-registry/hub-api
npm ci
npm run build
npm run lint

# Live (after edge redeploy completes)
.\testnet\hub-explorer-smoke.ps1 -BaseDomain testnet.cregnet.dev
.\testnet\public-alpha-rehearsal.ps1 -Execute -BaseDomain testnet.cregnet.dev
```
