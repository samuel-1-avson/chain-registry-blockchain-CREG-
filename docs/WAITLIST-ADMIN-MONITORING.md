# Waitlist admin monitoring (owner-only)

Detailed waitlist traffic and registration metrics are **not** exposed in the public SPA or team dashboards. Only the project owner (you) can read them via a secret token and Firebase/Google Cloud console access.

## What stays public

- `waitlistPublicStats` — total count and redacted recent ticker (`addressPreview` like `0x1234...abcd`) on the live site.
- No third-party analytics (no GA4, PostHog, etc.).

## What is private (owner-only)

| Signal | Where it lives | How you read it |
|--------|----------------|-----------------|
| Registration audit events | Cloud Logging | Logs Explorer filter (below) |
| Counts, today/7d registrations, recent list, page views | `waitlistAdminStats` HTTP function | `curl` with `ADMIN_STATS_TOKEN` |
| Anonymous page views | Firestore `stats/traffic` | Via admin stats only (clients cannot read) |

## Firebase project access

Restrict Firebase / GCP access so only your Google account can view logs, Firestore, and Functions:

1. Open [Firebase Console](https://console.firebase.google.com/) → project **gen-lang-client-0098858574** (numeric id `0098858574`).
2. **Project settings** → **Users and permissions** — remove broad team access; keep only your account as **Owner** (or minimal **Viewer** if you use a separate ops account).
3. In [Google Cloud Console IAM](https://console.cloud.google.com/iam-admin/iam?project=gen-lang-client-0098858574), avoid granting `roles/logging.viewer` or Firestore viewer roles to non-owner accounts.

## Set the admin stats token

Generate a long random secret locally (do not commit it):

```bash
# Example: 32-byte hex
openssl rand -hex 32
```

Store it as a Firebase Functions secret (from repo root or `Creg-waitlist/`):

```bash
cd Creg-waitlist
firebase functions:secrets:set ADMIN_STATS_TOKEN
# paste the value when prompted
```

Redeploy functions so the secret is bound:

```bash
cd functions
npm run build
npm run deploy
```

Local emulator (optional):

```bash
# functions/.env (gitignored) — see functions/.env.example
ADMIN_STATS_TOKEN=your-local-dev-token
```

## Admin stats API

**Endpoint:** `GET https://us-central1-gen-lang-client-0098858574.cloudfunctions.net/waitlistAdminStats`

**Auth:** prefer header; query param is a fallback.

```bash
export ADMIN_STATS_TOKEN="your-secret-here"

# Production
curl -sS \
  -H "Authorization: Bearer $ADMIN_STATS_TOKEN" \
  "https://us-central1-gen-lang-client-0098858574.cloudfunctions.net/waitlistAdminStats" \
  | jq .

# Local emulator (default port 5001)
curl -sS \
  -H "Authorization: Bearer $ADMIN_STATS_TOKEN" \
  "http://127.0.0.1:5001/gen-lang-client-0098858574/us-central1/waitlistAdminStats" \
  | jq .
```

**Response shape:**

```json
{
  "count": 42,
  "registrationsToday": 3,
  "registrationsLast7d": 18,
  "roleCounts": {
    "validators": 40,
    "publishers": 1,
    "securityAudits": 1
  },
  "recentRegistrations": [
    {
      "position": 42,
      "role": "Validator Node",
      "tier": "alpha",
      "addressPreview": "0x1234...abcd",
      "registeredAt": "2026-06-15T12:34:56.789Z"
    }
  ],
  "pageViewsToday": 120,
  "pageViewsTotal": 4500,
  "updatedAt": "2026-06-15T13:00:00.000Z"
}
```

No full wallet addresses are returned. CORS is not enabled on this endpoint (curl / server-side only).

## Registration audit log (Cloud Logging)

Each successful registration emits a structured log line:

```json
{
  "event": "waitlist_registration",
  "position": 42,
  "tier": "alpha",
  "role": "Validator Node",
  "walletPrefix": "0x1234...abcd"
}
```

**Logs Explorer filter** ([open in GCP](https://console.cloud.google.com/logs/query?project=gen-lang-client-0098858574)):

```
resource.type="cloud_run_revision"
resource.labels.service_name="waitlistadminstats"
OR resource.labels.service_name="registerwaitlist"
jsonPayload.event="waitlist_registration"
```

Broader filter for all registration JSON logs:

```
resource.type="cloud_run_revision"
textPayload=~"waitlist_registration"
```

Or for 2nd gen functions:

```
resource.type="cloud_function"
OR resource.type="cloud_run_revision"
textPayload=~"\"event\":\"waitlist_registration\""
```

## Page views

The SPA calls `waitlistTrackView` once per browser session (`sessionStorage` key `creg_pv`). Counts are stored in Firestore `stats/traffic` and are **not** readable from client SDK (rules only allow public read on `stats/waitlist`).

Optional override URL in `.env`:

```
VITE_WAITLIST_TRACK_VIEW_URL=https://us-central1-gen-lang-client-0098858574.cloudfunctions.net/waitlistTrackView
```

## Deploy

From `Creg-waitlist/functions`:

```bash
npm run build
npm run deploy
```

Deploys: `registerWaitlist`, `waitlistPublicStats`, `waitlistAdminStats`, `waitlistTrackView`.

After deploy, set `VITE_WAITLIST_STATS_URL` / `VITE_WAITLIST_TRACK_VIEW_URL` in production hosting env if you use custom domains or non-default URLs.

## Security notes

- Rotate `ADMIN_STATS_TOKEN` if leaked: `firebase functions:secrets:set ADMIN_STATS_TOKEN` and redeploy.
- Do not build a public admin UI or share the token in client env vars.
- `waitlistTrackView` is intentionally unauthenticated but only increments anonymous counters; detailed metrics require the admin token.

## Registration integrity controls

The public role counters report unique signed wallet claims for `Validator Node`, `Publisher`, and `Security Audits`. They do not establish unique human identity: one person can control multiple valid wallets.

Current practical controls are:

- EIP-191 proof of wallet control, bound to the waitlist domain and selected role.
- Single-use, expiring registration nonces.
- Case-insensitive wallet uniqueness through server-only hashed `wallet_claims` documents.
- reCAPTCHA Enterprise rejection below the configured score (production default `0.5`).
- A maximum of three signed, bot-checked wallet registrations per network per rolling 24 hours.
- Server-only registrations, claims, nonces, and rate-limit collections.

For stronger one-person/one-registration enforcement, add a separately reviewed proof-of-personhood mechanism such as verified email/phone or an external identity provider. Wallet signatures alone cannot provide that guarantee.

### Existing-data audit snapshot (2026-06-23)

A read-only audit found 3,291 registration documents representing 3,289 case-insensitive wallet addresses. One wallet appeared in three legacy documents, so the wallet-claim backfill collapses two duplicates without deleting registration history. The dataset is also highly bursty: 1,248 registrations occurred on 2026-06-19, including 389 in one hour. Treat the current population as wallet registrations—not verified people—and review these bursts before using them for governance, rewards, or validator allocation.
