# HOSTING-301 - Public HTTPS on GCP (gcloud SDK + Cloudflare)

**Domain:** `cregnet.dev` | **BaseDomain:** `testnet.cregnet.dev`

**Goal:** Serve the 3-node Sepolia lab on public HTTPS so `chain-spec.sepolia.json` matches live URLs.

**Budget & billing guardrails:** [GCP-BUDGET-ARCHITECTURE.md](../docs/GCP-BUDGET-ARCHITECTURE.md) ($150 target / $175 envelope).

All workstation commands assume:

```powershell
cd F:\project\chain-registry\chain-registry
```

---

## 0. One-time setup

### Google Cloud SDK

You already have `gcloud`. Confirm login and project:

```powershell
gcloud auth login
gcloud config set project gen-lang-client-0022105784
gcloud auth application-default login   # optional, for some tools
```

### Hosting config

```powershell
Copy-Item testnet\gcp\hosting.env.example testnet\gcp\hosting.env
# Edit hosting.env if project/zone/email differ
```

### Cloudflare API token (optional, for DNS automation)

1. [Cloudflare API Tokens](https://dash.cloudflare.com/profile/api-tokens) -> **Create Token**
2. Template: **Edit zone DNS** for zone `cregnet.dev`
3. Set in PowerShell before DNS step:

```powershell
$env:CF_API_TOKEN = "your-token"
```

Or add `CF_API_TOKEN=...` to `testnet/gcp/hosting.env` (gitignored).

---

## 1. Orchestrated path (recommended)

Preview:

```powershell
.\testnet\gcp\run-hosting-301.ps1 -Step check
```

Create VM + static IP (costs apply; requires `-Confirm`):

```powershell
.\testnet\gcp\run-hosting-301.ps1 -Step provision -Confirm
```

Full pipeline (provision -> patch spec -> DNS -> deploy -> verify):

```powershell
.\testnet\gcp\run-hosting-301.ps1 -Step all -Confirm
```

If you prefer manual DNS in the Cloudflare UI, skip API:

```powershell
.\testnet\gcp\run-hosting-301.ps1 -Step all -Confirm -SkipDns
```

---

## 2. Step-by-step (manual control)

| Step | Command | What it does |
|------|---------|--------------|
| 1 | `.\testnet\gcp\provision-vm.ps1 -Confirm` | Static IP, firewall 22/80/443, Ubuntu VM + Docker bootstrap |
| 2 | `.\testnet\gcp\run-hosting-301.ps1 -Step prep` | Patch/sign chain spec + `sepolia-3node.env` |
| 3 | `.\testnet\gcp\set-cloudflare-dns.ps1 -StaticIp <IP>` | Five A records, **proxied off** |
| 4 | Wait 2-10 min | DNS propagation |
| 5 | `.\testnet\gcp\deploy-stack.ps1 -PushEnv` | `git clone`, `docker compose up`, Caddy TLS |
| 6 | `.\testnet\hosting-301-verify.ps1 -BaseDomain testnet.cregnet.dev` | HOSTING-301 acceptance |

### Cloudflare DNS (manual)

Zone: **cregnet.dev** | Type **A** | **DNS only** (grey cloud) | Content = GCP static IP

| Name | FQDN |
|------|------|
| `api.testnet` | `api.testnet.cregnet.dev` |
| `explorer.testnet` | `explorer.testnet.cregnet.dev` |
| `faucet.testnet` | `faucet.testnet.cregnet.dev` |
| `spec.testnet` | `spec.testnet.cregnet.dev` |
| `ipfs.testnet` | `ipfs.testnet.cregnet.dev` |

[Cloudflare DNS dashboard](https://dash.cloudflare.com/) -> `cregnet.dev` -> DNS -> Records

---

## 3. gcloud cheat sheet

```powershell
# SSH into VM
.\testnet\gcp\ssh-vm.ps1

# Run remote command
.\testnet\gcp\ssh-vm.ps1 -Command "docker ps"

# VM status / IP
gcloud compute instances list
gcloud compute addresses list --regions=us-central1

# Caddy / TLS logs
.\testnet\gcp\ssh-vm.ps1 -Command "docker logs -f creg-3node-caddy"

# Re-upload env after local changes
.\testnet\gcp\push-env.ps1
.\testnet\gcp\deploy-stack.ps1

# Tear down (when finished)
gcloud compute instances delete creg-testnet-vm --zone=us-central1-a --quiet
gcloud compute addresses delete creg-testnet-ip --region=us-central1 --quiet
```

---

## 4. What runs on the VM

`start-3node-gcp.sh` brings up:

- `docker-compose.3node.yml` - 3 validators
- `docker-compose.3node-services.yml` - explorer, faucet, spec-server, IPFS
- `docker-compose.3node-ingress.yml` - Caddy + Let's Encrypt

Public URLs:

- https://api.testnet.cregnet.dev
- https://explorer.testnet.cregnet.dev
- https://faucet.testnet.cregnet.dev
- https://spec.testnet.cregnet.dev
- https://ipfs.testnet.cregnet.dev

---

## 5. Troubleshooting

| Symptom | Fix |
|---------|-----|
| Caddy ACME fails | Grey-cloud DNS; port 80 open; wait for propagation |
| `gcloud compute ssh` permission | `gcloud auth login`; check project IAM (Compute Admin) |
| Docker not ready on first deploy | Wait 2-3 min after provision; re-run `deploy-stack.ps1` |
| 502 on API | `ssh-vm` -> `docker compose ... ps`; check `creg-node-3` |
| Spec URL mismatch | Re-run `prepare-public-hosting.ps1` with correct `-StaticIp` |

---

## References

- Scripts: `testnet/gcp/`
- [OPERATOR.md](./OPERATOR.md)
- [docs/NEXT_WORK.md](../../docs/NEXT_WORK.md)
