# Waitlist — Firebase backend deploy

Registration uses the callable Cloud Function **`registerWaitlist`**. Browsers cannot write to Firestore directly.

**Cost / architecture:** [GCP-BUDGET-ARCHITECTURE.md](./GCP-BUDGET-ARCHITECTURE.md)

## Prerequisites

- Firebase CLI (`npx firebase-tools@latest`)
- `firebase login`
- **Blaze billing** on `gen-lang-client-0098858574`
- Secret Manager API enabled
- reCAPTCHA v3 site key + secret key

## One-time setup

```powershell
cd Creg-waitlist

# Secret (paste reCAPTCHA *secret* when prompted — name is RECAPTCHA_SECRET_KEY, not the value)
"YOUR_RECAPTCHA_SECRET" | firebase functions:secrets:set RECAPTCHA_SECRET_KEY --project gen-lang-client-0098858574 --data-file -

# Client site key for production builds
copy .env.example .env
# Edit .env → VITE_RECAPTCHA_SITE_KEY=...
```

## Deploy

```powershell
cd chain-registry
.\testnet\gcp\deploy-waitlist-firebase.ps1
```

PowerShell must quote comma-separated targets:

```powershell
cd Creg-waitlist
npx --yes firebase-tools@latest deploy --project gen-lang-client-0098858574 --only "firestore,functions:registerWaitlist" --force
```

## Deploy static site (same VM as testnet)

```powershell
cd chain-registry
.\testnet\gcp\deploy-waitlist.ps1 -Confirm -SkipDns
```

Requires `Creg-waitlist/.env` with `VITE_RECAPTCHA_SITE_KEY` before build.

## Verify

1. https://waitlist.cregnet.dev/ loads
2. Connect wallet → select role → register
3. Counter increments; duplicate wallet rejected
4. Direct Firestore writes from browser console → permission denied

## Key IDs

| Item | Value |
|------|--------|
| Firebase project | `gen-lang-client-0098858574` |
| Named Firestore DB | `ai-studio-6b167dc8-a078-4526-a86b-de2a8722a753` |
| Function | `registerWaitlist` (`us-central1`) |
| Static host | `waitlist.cregnet.dev` |
