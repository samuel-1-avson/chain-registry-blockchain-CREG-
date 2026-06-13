# Waitlist — Firebase backend deploy

Registration uses the callable Cloud Function **`registerWaitlist`**. Browsers cannot write to Firestore directly.

**Cost / architecture:** [GCP-BUDGET-ARCHITECTURE.md](./GCP-BUDGET-ARCHITECTURE.md)

## Prerequisites

- Firebase CLI (`npx firebase-tools@latest`)
- `firebase login`
- **Blaze billing** on `gen-lang-client-0098858574`
- [reCAPTCHA Enterprise](https://cloud.google.com/recaptcha-enterprise) key (score / website integration)
- `recaptchaenterprise.googleapis.com` API enabled

## One-time setup

```powershell
cd Creg-waitlist

# Client site key for production builds (Enterprise key from Cloud Console / gcloud)
copy .env.example .env
# Edit .env → VITE_RECAPTCHA_SITE_KEY=...
```

Grant the function runtime service account reCAPTCHA Enterprise access:

```powershell
gcloud projects add-iam-policy-binding gen-lang-client-0098858574 `
  --member="serviceAccount:913012834874-compute@developer.gserviceaccount.com" `
  --role="roles/recaptchaenterprise.agent"
```

The function verifies tokens with the **Enterprise Assessments API** (site key is configured in `functions/src/index.ts`). No classic reCAPTCHA secret key is required.

Ensure the Cloud Run service allows public callable invocations (Firebase deploy sets `invoker: "public"`; if registration returns 403 Unauthenticated, run):

```powershell
gcloud run services add-iam-policy-binding registerwaitlist `
  --region=us-central1 `
  --project=gen-lang-client-0098858574 `
  --member="allUsers" `
  --role="roles/run.invoker"
```

## Deploy

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
2. Connect wallet → select role → register (success shows queue position + tier)
3. Duplicate wallet → rejected
4. Direct Firestore writes from browser console → permission denied

## Key IDs

| Item | Value |
|------|--------|
| Firebase project | `gen-lang-client-0098858574` |
| Named Firestore DB | `ai-studio-6b167dc8-a078-4526-a86b-de2a8722a753` |
| Function | `registerWaitlist` (`us-central1`, public invoker) |
| reCAPTCHA site key | `6LfLaB0tAAAAAD-YmiBUrSNzEsybw6C-jWSxDSqM` (`creg-waitlist`) |
| Static host | `waitlist.cregnet.dev` |
| Waitlist app repo | [github.com/samuel-1-avson/Creg-waitlist](https://github.com/samuel-1-avson/Creg-waitlist) |
