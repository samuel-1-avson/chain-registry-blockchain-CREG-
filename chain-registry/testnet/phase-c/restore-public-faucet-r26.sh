#!/usr/bin/env bash
# Restore the real Sepolia faucet page after reconciling the production ledger.
# All token and native claims remain disabled until governed policy is approved.
set -euo pipefail
exec 9>/var/lock/creg-public-faucet-rollout.lock
flock -n 9 || { echo 'Another public faucet rollout is active' >&2; exit 1; }

image=${CREG_FAUCET_PUBLIC_IMAGE:?Set a digest-pinned faucet image}
[[ "$image" =~ ^us-central1-docker\.pkg\.dev/gen-lang-client-0022105784/creg-testnet/creg-faucet@sha256:[0-9a-f]{64}$ ]] || {
  echo 'The faucet image must use the approved registry and a sha256 digest' >&2
  exit 1
}

network=testnet_creg-unified-net
db=creg-unified-faucet-db
old_app=creg-faucet-public-r25
public_app=creg-faucet-public-r26
private_app=creg-faucet-public-r26-preflight
base=/var/lib/creg/public-faucet-r25
work=/var/lib/creg/public-faucet-r26
manifest=/home/samue/creg-r7-deployment-manifest.json
compose=/home/samue/docker-compose.public-faucet.yml
status_file=$(mktemp /tmp/creg-faucet-r26-status.XXXXXX)
page_file=$(mktemp /tmp/creg-faucet-r26-page.XXXXXX)
trap 'rm -f "$status_file" "$page_file"' EXIT

for path in "$base/faucet.env" "$base/postgres.env" "$manifest" "$compose"; do
  sudo test -s "$path" || { echo "Required rollout input is missing: $path" >&2; exit 1; }
done
[[ "$(sudo docker inspect --format '{{.State.Health.Status}}' "$db")" == healthy ]] || {
  echo 'Production faucet PostgreSQL is not healthy' >&2; exit 1;
}
[[ "$(sudo docker inspect --format '{{.State.Running}}' "$old_app")" == false ]] || {
  echo 'The old faucet unexpectedly started; inspect it before rollout' >&2; exit 1;
}
sudo docker network inspect "$network" >/dev/null
if sudo docker inspect "$private_app" >/dev/null 2>&1; then
  echo 'A previous private rollout container exists; inspect it before rerunning' >&2
  exit 1
fi
if sudo docker inspect "$public_app" >/dev/null 2>&1; then
  echo 'The R26 public container already exists; inspect it before rerunning' >&2
  exit 1
fi

if ! sudo grep -Fq -- "$image" "$compose"; then
  echo 'The production Compose file does not pin the requested faucet image' >&2; exit 1;
fi

db_id_before=$(sudo docker inspect --format '{{.Id}}' "$db")
volume_mount_before=$(sudo docker volume inspect --format '{{.Mountpoint}}' testnet_faucet-postgres-data)
caddy_ip=$(sudo docker inspect creg-unified-caddy --format '{{range .NetworkSettings.Networks}}{{.IPAddress}}{{end}}')
sudo install -d -m 700 "$work"

# Keep a root-only logical backup before the new app creates ledger tables.
backup="$work/production-before-reconciliation-$(date -u +%Y%m%dT%H%M%SZ).dump"
sudo docker exec "$db" sh -c 'pg_dump -Fc -U "$POSTGRES_USER" -d "$POSTGRES_DB"' | sudo tee "$backup" >/dev/null
sudo chmod 600 "$backup"
sudo test -s "$backup" || { echo 'Production database backup is empty' >&2; exit 1; }
backup_sha256=$(sudo sha256sum "$backup" | cut -d ' ' -f 1)

# Preserve the protected signing key and database URL. Remove all allocation
# values so the source-level policy gate cannot authorize token claims.
sudo python3 - "$base/faucet.env" "$work/faucet.env" "$manifest" "$caddy_ip" <<'PY'
import ipaddress
import json
import os
import secrets
import sys
from urllib.parse import urlsplit

source, target, manifest_path, caddy_ip = sys.argv[1:]
ipaddress.ip_address(caddy_ip)
with open(source, encoding="utf-8") as stream:
    lines = stream.readlines()
with open(manifest_path, encoding="utf-8") as stream:
    manifest = json.load(stream)
if str(manifest["chainId"]) != "11155111":
    raise SystemExit("The R7 deployment manifest is not Sepolia")
if manifest["cregToken"].lower() != "0xbc2a72107c6765364d0a822a88a6e6c61f30eaa6":
    raise SystemExit("The R7 deployment manifest has the wrong token")

values = {}
for line in lines:
    text = line.strip()
    if text and not text.startswith("#") and "=" in text:
        key, value = text.split("=", 1)
        values[key] = value
for key in ("FAUCET_PG_URL", "FAUCET_ADDRESS", "FAUCET_PRIVATE_KEY"):
    if not values.get(key):
        raise SystemExit("Protected faucet config lacks " + key)
pg_host = urlsplit(values["FAUCET_PG_URL"]).hostname
if pg_host not in {"faucet-db", "creg-unified-faucet-db"}:
    raise SystemExit("Production faucet database URL points outside its Docker network")
if values.get("FAUCET_CHAIN_ID", "11155111") != "11155111":
    raise SystemExit("Protected faucet config is not Sepolia")
if values.get("FAUCET_TOKEN_CONTRACT", manifest["cregToken"]).lower() != manifest["cregToken"].lower():
    raise SystemExit("Protected faucet token does not match the R7 manifest")

removed = {
    "FAUCET_TOKEN_ALLOCATION_WEI", "FAUCET_TOKEN_DAILY_CAP_WEI",
    "FAUCET_TOKEN_ADDRESS_LIFETIME_CAP_WEI", "FAUCET_TOKEN_HOT_WALLET_FLOAT_WEI",
    "FAUCET_LEDGER_RECONCILED", "FAUCET_TOKEN_CLAIMS_ENABLED",
}
overrides = {
    "FAUCET_CHAIN_ID": "11155111",
    "FAUCET_TOKEN_CONTRACT": manifest["cregToken"],
    "FAUCET_TOKEN_DEPLOYMENT_TX_HASH": manifest["cregTokenDeploymentTxHash"],
    "FAUCET_TOKEN_DEPLOYMENT_BLOCK": str(manifest["cregTokenDeploymentBlock"]),
    "FAUCET_RECONCILIATION_RPC_URL": "https://api.routescan.io/v2/network/testnet/evm/11155111/etherscan/api",
    "FAUCET_RECONCILIATION_INDEXER_URL": "https://api.routescan.io/v2/network/testnet/evm/11155111/etherscan/api",
    "FAUCET_DEPLOYMENT_RECEIPT_RPC_URL": "https://1rpc.io/sepolia",
    "FAUCET_RECONCILIATION_CONFIRMATIONS": "64",
    "FAUCET_NATIVE_DRIP_AMOUNT": "0",
    "FAUCET_TRUSTED_PROXY_CIDRS": caddy_ip + "/32",
    "FAUCET_ADMIN_TOKEN": secrets.token_urlsafe(48),
}
output = []
seen = set()
for line in lines:
    text = line.strip()
    if not text or text.startswith("#") or "=" not in text:
        output.append(line)
        continue
    key, _ = text.split("=", 1)
    if key in removed or key in seen:
        continue
    output.append(key + "=" + overrides.get(key, values[key]) + "\n")
    seen.add(key)
for key, value in overrides.items():
    if key not in seen:
        output.append(key + "=" + value + "\n")
with open(target, "w", encoding="utf-8") as stream:
    stream.writelines(output)
os.chmod(target, 0o600)
print("Protected R26 config prepared for Sepolia with all claims disabled")
PY

sudo docker compose -f "$compose" config --quiet
sudo docker pull "$image" >/dev/null
sudo docker run -d --name "$private_app" --network "$network" \
  --env-file "$work/faucet.env" -p 127.0.0.1:18092:8082 "$image" >/dev/null

admin_status() {
  sudo docker exec -i "$1" sh -c 'printf "Authorization: Bearer %s\n" "$FAUCET_ADMIN_TOKEN" | curl --max-time 6 -fsS -H @- http://127.0.0.1:8082/admin/status' \
    >"$status_file" 2>/dev/null
}
await_reconciliation() {
  local app=$1
  for attempt in $(seq 1 90); do
    if admin_status "$app" && python3 - "$status_file" <<'PY'
import json, sys
s = json.load(open(sys.argv[1], encoding="utf-8"))
assert s.get("ledger_reconciled") is True
assert s.get("token_claims_enabled") is False
PY
    then return 0; fi
    sleep 5
  done
  return 1
}

await_reconciliation "$private_app" || {
  echo 'Production database reconciliation did not pass in private preflight' >&2; exit 1;
}
sudo docker restart "$private_app" >/dev/null
await_reconciliation "$private_app" || {
  echo 'Production ledger did not recover after the private process restart' >&2; exit 1;
}
sudo docker exec -i "$private_app" sh -c 'printf "Authorization: Bearer %s\n" "$FAUCET_ADMIN_TOKEN" | curl --max-time 6 -fsS -H @- http://127.0.0.1:8082/admin/recovery' \
  | python3 -c 'import json,sys; r=json.load(sys.stdin); assert r.get("status")=="clear" and not r.get("claims") and r.get("token_claims_enabled") is False'
curl --max-time 9 -fsS http://127.0.0.1:18092/health \
  | python3 -c 'import json,sys; h=json.load(sys.stdin); assert h.get("ledger_reconciled") is True and h.get("token_claims_enabled") is False'
sudo docker stop "$private_app" >/dev/null
sudo docker rm "$private_app" >/dev/null

# Compose replaces only the faucet app. --no-deps leaves PostgreSQL and its
# existing persistent volume in place.
sudo docker compose -f "$compose" up -d --no-deps faucet >/dev/null
[[ "$(sudo docker inspect --format '{{.Id}}' "$db")" == "$db_id_before" ]]
[[ "$(sudo docker volume inspect --format '{{.Mountpoint}}' testnet_faucet-postgres-data)" == "$volume_mount_before" ]]

public_ok=false
for attempt in $(seq 1 30); do
  if curl --max-time 10 -fsS https://faucet.testnet.cregnet.dev/health \
      | python3 -c 'import json,sys; h=json.load(sys.stdin); assert h.get("ledger_reconciled") is True and h.get("token_claims_enabled") is False' 2>/dev/null; then
    public_ok=true
    break
  fi
  sleep 2
done
[[ "$public_ok" == true ]] || { echo 'Public faucet health did not pass after Compose rollout' >&2; exit 1; }
curl --max-time 10 -fsS https://faucet.testnet.cregnet.dev/ -o "$page_file"
grep -q 'CREG Testnet Faucet' "$page_file"
grep -q 'Claims Paused' "$page_file"
curl --max-time 20 -fsS https://faucet.testnet.cregnet.dev/api/stats \
  | python3 -c 'import json,sys; s=json.load(sys.stdin); assert s.get("status")=="available" and s.get("token_claims_enabled") is False and s.get("native_drip_amount")=="0"'
curl --max-time 10 -fsS https://faucet.testnet.cregnet.dev/api/network \
  | python3 -c 'import json,sys; n=json.load(sys.stdin); assert n.get("chain_id")==11155111 and n.get("token_contract", "").lower()=="0xbc2a72107c6765364d0a822a88a6e6c61f30eaa6"'

ledger=$(sudo docker exec "$db" sh -c 'psql -U "$POSTGRES_USER" -d "$POSTGRES_DB" -Atc "select reconciled_through_block||chr(124)||faucet_balance_wei||chr(124)||total_supply_wei from faucet_reconciliation_state; select count(*) from faucet_token_transfers; select count(*) from faucet_funding_ledger"')
[[ -n "$ledger" ]] || { echo 'Production reconciliation rows are unavailable' >&2; exit 1; }
sudo python3 - "$work/report.json" "$image" "$backup" "$backup_sha256" "$ledger" <<'PY'
import json, sys
from datetime import datetime, timezone

path, image, backup, backup_sha256, ledger = sys.argv[1:]
rows = ledger.splitlines()
watermark, balance, supply = rows[0].split("|")
report = {
    "verified_at_utc": datetime.now(timezone.utc).isoformat(),
    "public_url": "https://faucet.testnet.cregnet.dev/",
    "public_http_status": 200,
    "image": image,
    "chain_id": 11155111,
    "token_contract": "0xbC2a72107C6765364D0A822a88A6e6C61f30EAa6",
    "production_ledger_reconciled": True,
    "restart_recovery_confirmed": True,
    "token_claims_enabled": False,
    "native_drips_enabled": False,
    "reconciled_through_block": int(watermark),
    "faucet_balance_wei": balance,
    "total_supply_wei": supply,
    "canonical_transfer_rows": int(rows[1]),
    "funding_rows": int(rows[2]),
    "database_backup": backup,
    "database_backup_sha256": backup_sha256,
    "database_container_and_volume_preserved": True,
}
with open(path, "w", encoding="utf-8") as stream:
    json.dump(report, stream, indent=2)
    stream.write("\n")
print(json.dumps(report, indent=2))
PY
sudo chmod 600 "$work/faucet.env" "$backup"
