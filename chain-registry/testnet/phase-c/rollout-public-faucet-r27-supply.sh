#!/usr/bin/env bash
# Publish the real single-block Sepolia supply breakdown after private checks.
# This rollout cannot enable token claims or native ETH drips.
set -euo pipefail
exec 9>/var/lock/creg-public-faucet-rollout.lock
flock -n 9 || { echo 'Another public faucet rollout is active' >&2; exit 1; }

image=${CREG_FAUCET_PUBLIC_IMAGE:?Set the digest-pinned R27 faucet image}
[[ "$image" =~ ^us-central1-docker\.pkg\.dev/gen-lang-client-0022105784/creg-testnet/creg-faucet@sha256:[0-9a-f]{64}$ ]] || {
  echo 'The image must come from the approved registry and have a sha256 digest' >&2
  exit 1
}

network=testnet_creg-unified-net
db=creg-unified-faucet-db
old_app=creg-faucet-public-r26
public_app=creg-faucet-public-r27
private_app=creg-faucet-public-r27-preflight
source_env=/var/lib/creg/public-faucet-r26/faucet.env
work=/var/lib/creg/public-faucet-r27
manifest=/home/samue/creg-r7-deployment-manifest.json
compose=/home/samue/docker-compose.public-faucet.yml
supply_file=$(mktemp /tmp/creg-faucet-r27-supply.XXXXXX)
page_file=$(mktemp /tmp/creg-faucet-r27-page.XXXXXX)
trap 'rm -f "$supply_file" "$page_file"' EXIT

for path in "$source_env" "$manifest" "$compose"; do
  sudo test -s "$path" || { echo "Required rollout input is missing: $path" >&2; exit 1; }
done
[[ "$(sudo docker inspect --format '{{.State.Health.Status}}' "$db")" == healthy ]]
[[ "$(sudo docker inspect --format '{{.State.Health.Status}}' "$old_app")" == healthy ]]
sudo docker network inspect "$network" >/dev/null
if sudo docker inspect "$private_app" >/dev/null 2>&1 || sudo docker inspect "$public_app" >/dev/null 2>&1; then
  echo 'An R27 container already exists; inspect it before rerunning' >&2
  exit 1
fi
if ! sudo grep -Fq -- "$image" "$compose"; then
  echo 'The public Compose file does not pin the requested R27 image' >&2
  exit 1
fi

db_id_before=$(sudo docker inspect --format '{{.Id}}' "$db")
volume_mount_before=$(sudo docker volume inspect --format '{{.Mountpoint}}' testnet_faucet-postgres-data)
sudo install -d -m 700 "$work"
backup="$work/production-before-r27-$(date -u +%Y%m%dT%H%M%SZ).dump"
sudo docker exec "$db" sh -c 'pg_dump -Fc -U "$POSTGRES_USER" -d "$POSTGRES_DB"' | sudo tee "$backup" >/dev/null
sudo chmod 600 "$backup"
sudo test -s "$backup" || { echo 'Production database backup is empty' >&2; exit 1; }
backup_sha256=$(sudo sha256sum "$backup" | cut -d ' ' -f 1)

sudo python3 - "$source_env" "$work/faucet.env" "$manifest" <<'PY'
import json
import os
import secrets
import sys

source, target, manifest_path = sys.argv[1:]
with open(source, encoding="utf-8") as stream:
    lines = stream.readlines()
with open(manifest_path, encoding="utf-8") as stream:
    manifest = json.load(stream)
if str(manifest["chainId"]) != "11155111":
    raise SystemExit("The deployment manifest is not Sepolia")
if manifest["cregToken"].lower() != "0xbc2a72107c6765364d0a822a88a6e6c61f30eaa6":
    raise SystemExit("The manifest has an unexpected token")
values = {}
for line in lines:
    text = line.strip()
    if text and not text.startswith("#") and "=" in text:
        key, value = text.split("=", 1)
        values[key] = value
if values.get("FAUCET_CHAIN_ID") != "11155111" or values.get("FAUCET_TOKEN_CONTRACT", "").lower() != manifest["cregToken"].lower():
    raise SystemExit("The current public faucet does not match the R7 manifest")
if values.get("FAUCET_NATIVE_DRIP_AMOUNT") != "0":
    raise SystemExit("The current public faucet unexpectedly enables native drips")
removed = {
    "FAUCET_TOKEN_ALLOCATION_WEI", "FAUCET_TOKEN_DAILY_CAP_WEI",
    "FAUCET_TOKEN_ADDRESS_LIFETIME_CAP_WEI", "FAUCET_TOKEN_HOT_WALLET_FLOAT_WEI",
    "FAUCET_LEDGER_RECONCILED", "FAUCET_TOKEN_CLAIMS_ENABLED",
}
overrides = {
    "FAUCET_TREASURY_ADDRESS": manifest["deployer"],
    "FAUCET_GOVERNANCE_ADDRESS": manifest["governance"],
    "FAUCET_STAKING_ADDRESS": manifest["staking"],
    "FAUCET_NATIVE_DRIP_AMOUNT": "0",
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
print("R27 config prepared with supply holders and all claims disabled")
PY

sudo docker compose -f "$compose" config --quiet
sudo docker pull "$image" >/dev/null
sudo docker run -d --name "$private_app" --network "$network" \
  --env-file "$work/faucet.env" -p 127.0.0.1:18093:8082 "$image" >/dev/null

check_supply() {
  local url=$1
  curl --max-time 20 -fsS "$url" >"$supply_file" 2>/dev/null &&
    python3 - "$supply_file" <<'PY'
import json, sys
s = json.load(open(sys.argv[1], encoding="utf-8"))
unit = 10 ** 18
assert s.get("status") == "available" and s.get("chain_id") == 11155111
assert s.get("token_claims_enabled") is False
assert s.get("total_supply_wei") == str(20_000_000 * unit)
assert s.get("treasury", {}).get("balance_wei") == str(19_989_590 * unit)
assert s.get("faucet", {}).get("balance_wei") == str(8_000 * unit)
assert s.get("governance", {}).get("balance_wei") == "0"
assert s.get("staking", {}).get("balance_wei") == str(410 * unit)
assert s.get("other_holders_wei") == str(2_000 * unit)
assert int(s["block_number"]) > 11_809_000
PY
}

await_supply() {
  local url=$1
  for attempt in $(seq 1 35); do
    if check_supply "$url" 2>/dev/null; then return 0; fi
    sleep 5
  done
  return 1
}
await_supply http://127.0.0.1:18093/api/supply || {
  echo 'Private R27 supply snapshot failed its real Sepolia accounting gate' >&2; exit 1;
}
sudo docker restart "$private_app" >/dev/null
await_supply http://127.0.0.1:18093/api/supply || {
  echo 'Private R27 supply snapshot did not recover after restart' >&2; exit 1;
}
curl --max-time 12 -fsS http://127.0.0.1:18093/health \
  | python3 -c 'import json,sys; h=json.load(sys.stdin); assert h.get("ledger_reconciled") is True and h.get("token_claims_enabled") is False'
sudo docker stop "$private_app" >/dev/null
sudo docker rm "$private_app" >/dev/null

sudo docker compose -f "$compose" up -d --no-deps faucet >/dev/null
[[ "$(sudo docker inspect --format '{{.Id}}' "$db")" == "$db_id_before" ]]
[[ "$(sudo docker volume inspect --format '{{.Mountpoint}}' testnet_faucet-postgres-data)" == "$volume_mount_before" ]]
public_ok=false
for attempt in $(seq 1 30); do
  if check_supply https://faucet.testnet.cregnet.dev/api/supply 2>/dev/null; then
    public_ok=true
    break
  fi
  sleep 2
done
[[ "$public_ok" == true ]] || { echo 'Public R27 supply route did not pass after rollout' >&2; exit 1; }
curl --max-time 10 -fsS -o "$page_file" https://faucet.testnet.cregnet.dev/
grep -q 'Where the tCREG supply is held' "$page_file"
curl --max-time 12 -fsS https://faucet.testnet.cregnet.dev/health \
  | python3 -c 'import json,sys; h=json.load(sys.stdin); assert h.get("ledger_reconciled") is True and h.get("token_claims_enabled") is False'
curl --max-time 12 -fsS https://faucet.testnet.cregnet.dev/api/stats \
  | python3 -c 'import json,sys; s=json.load(sys.stdin); assert s.get("token_claims_enabled") is False and s.get("native_drip_amount") == "0"'

sudo python3 - "$work/report.json" "$image" "$backup" "$backup_sha256" "$supply_file" <<'PY'
import json, sys
from datetime import datetime, timezone

path, image, backup, backup_sha256, supply_path = sys.argv[1:]
supply = json.load(open(supply_path, encoding="utf-8"))
report = {
    "verified_at_utc": datetime.now(timezone.utc).isoformat(),
    "public_url": "https://faucet.testnet.cregnet.dev/",
    "public_http_status": 200,
    "image": image,
    "production_database_container_and_volume_preserved": True,
    "production_database_backup": backup,
    "production_database_backup_sha256": backup_sha256,
    "private_restart_recovery_confirmed": True,
    "requests_mocked": False,
    "on_chain_transaction_sent": False,
    "supply": supply,
}
with open(path, "w", encoding="utf-8") as stream:
    json.dump(report, stream, indent=2)
    stream.write("\n")
print(json.dumps(report, indent=2))
PY
