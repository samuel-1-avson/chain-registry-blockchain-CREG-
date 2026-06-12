# Waitlist Firebase audit checklist

**Scope:** `waitlist.cregnet.dev` (Firebase Hosting + Firestore). Backend rules and project config live outside this repo.

Use this checklist before widening waitlist traffic or changing collection schema.

---

## Firestore security rules

- [ ] Default deny: no wildcard `read, write: if true` on any collection.
- [ ] Waitlist writes require validated fields only (email format, max length, no arbitrary keys).
- [ ] Reads restricted to authenticated admin SDK / Cloud Functions — not public client reads of other users' rows.
- [ ] Rate limiting or App Check enabled for anonymous sign-ups if rules allow create.
- [ ] PII fields (email, name) not readable by other clients.

## Authentication and App Check

- [ ] Firebase App Check enforced for Hosting + Firestore in production.
- [ ] API keys restricted by HTTP referrer / bundle ID in Google Cloud Console.
- [ ] Service account keys not embedded in the static SPA bundle.

## Data retention and privacy

- [ ] Documented retention period for waitlist entries.
- [ ] Export/delete process defined (GDPR-style requests).
- [ ] No secrets, internal URLs, or operator keys stored in Firestore documents.

## Hosting and deployment

- [ ] `firebase.json` / hosting targets reviewed for cache headers on `index.html` (no stale PWA trapping users).
- [ ] Separate Firebase/GCP project from testnet edge (see `hosting.env.example` `GCP_WAITLIST_PROJECT`).
- [ ] CI deploy uses least-privilege deploy token; token not committed.

## Observability

- [ ] Firestore usage alerts (spike in writes/reads).
- [ ] Error reporting for client submission failures.
- [ ] Periodic export backup or BigQuery sync if waitlist is business-critical.

## Sign-off

| Role | Name | Date | Notes |
|------|------|------|-------|
| Engineering | | | |
| Security | | | |

---

*Repo action: re-run this checklist when Firebase rules, App Check, or collection schema change.*
