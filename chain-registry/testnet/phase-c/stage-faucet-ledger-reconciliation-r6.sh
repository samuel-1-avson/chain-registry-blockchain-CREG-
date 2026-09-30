#!/usr/bin/env bash
# Run a private faucet recovery candidate beside the live service. The candidate
# gets a snapshot of the real PostgreSQL data and real Sepolia RPC access, but
# uses a separate database volume and has all token/native claims disabled.
set -euo pipefail
exec 9>/var/lock/creg-faucet-ledger-stage.lock
flock -n 9 || { echo 'Another faucet ledger staging run is active' >&2; exit 1; }

image=${CREG_FAUCET_CANDIDATE_IMAGE:?set CREG_FAUCET_CANDIDATE_IMAGE to a digest-pinned image}
[[ "$image" == *@sha256:* ]] || { echo 'Candidate image must be pinned by sha256 digest' >&2; exit 1; }

live_faucet=creg-faucet-public-r25
live_db=creg-unified-faucet-db
network=testnet_creg-unified-net
base=/var/lib/creg/public-faucet-r25
candidate_dir=/var/lib/creg/faucet-ledger-reconciliation-r6
candidate_db=creg-faucet-ledger-stage-db-r6
candidate_app=creg-faucet-ledger-stage-r6
candidate_db_alias=faucet-ledger-stage-db-r6
candidate_volume=testnet_faucet-ledger-stage-r6-data
candidate_port=18088
pg_env="$base/postgres.env"
faucet_env="$base/faucet.env"
manifest=/home/samue/creg-r7-deployment-manifest.json

for path in "$pg_env" "$faucet_env"; do
  sudo test -s "$path" || { echo "Required protected config is unavailable: $path" >&2; exit 1; }
done
sudo test -s "$manifest" || { echo "R7 deployment manifest is unavailable: $manifest" >&2; exit 1; }
sudo docker inspect "$live_faucet" >/dev/null
sudo docker inspect "$live_db" >/dev/null
sudo docker network inspect "$network" >/dev/null
sudo docker inspect "$candidate_app" >/dev/null 2>&1 && { echo "Candidate container already exists: $candidate_app" >&2; exit 1; }
sudo docker inspect "$candidate_db" >/dev/null 2>&1 && { echo "Candidate database container already exists: $candidate_db" >&2; exit 1; }
sudo docker volume inspect "$candidate_volume" >/dev/null 2>&1 && { echo "Candidate database volume already exists: $candidate_volume" >&2; exit 1; }
if sudo test -e "$candidate_dir"; then
  leftovers=$(sudo find "$candidate_dir" -mindepth 1 -maxdepth 1 -printf '%f\n')
  [[ "$leftovers" == postgres.env ]] || { echo "Candidate config directory contains unexpected files: $candidate_dir" >&2; exit 1; }
  sudo rm -f -- "$candidate_dir/postgres.env"
  sudo rmdir -- "$candidate_dir"
fi

# Record live object IDs. The final check proves staging did not replace them.
live_faucet_id=$(sudo docker inspect --format '{{.Id}}' "$live_faucet")
live_db_id=$(sudo docker inspect --format '{{.Id}}' "$live_db")
live_db_image=$(sudo docker inspect --format '{{.Image}}' "$live_db")
live_volume_id=$(sudo docker volume inspect --format '{{.Mountpoint}}' testnet_faucet-postgres-data)

sudo install -d -m 700 "$candidate_dir"
sudo install -m 600 "$pg_env" "$candidate_dir/postgres.env"

# Generate a private app env file that points only at the cloned database and
# removes every token budget cap. A zero native drip also prevents gas sends.
sudo python3 - "$faucet_env" "$candidate_dir/faucet.env" "$candidate_db_alias" "$manifest" <<'PY'
import os
import sys
import json
from urllib.parse import urlsplit, urlunsplit

source, target, db_host, manifest_path = sys.argv[1:]
with open(source, encoding="utf-8") as stream:
    lines = stream.readlines()
with open(manifest_path, encoding="utf-8") as stream:
    manifest = json.load(stream)

network_config = {
    "FAUCET_CHAIN_ID": str(manifest["chainId"]),
    "FAUCET_TOKEN_CONTRACT": manifest["cregToken"],
    "FAUCET_TOKEN_DEPLOYMENT_TX_HASH": manifest["cregTokenDeploymentTxHash"],
    "FAUCET_TOKEN_DEPLOYMENT_BLOCK": str(manifest["cregTokenDeploymentBlock"]),
    "FAUCET_RECONCILIATION_RPC_URL": "https://ethereum-sepolia-rpc.publicnode.com",
    "FAUCET_RECONCILIATION_INDEXER_URL": "https://eth-sepolia.blockscout.com/api",
    "FAUCET_DEPLOYMENT_RECEIPT_RPC_URL": "https://1rpc.io/sepolia",
}
if network_config["FAUCET_CHAIN_ID"] != "11155111":
    raise SystemExit("R7 deployment manifest is not Sepolia")
if network_config["FAUCET_TOKEN_CONTRACT"].lower() != "0xbc2a72107c6765364d0a822a88a6e6c61f30eaa6":
    raise SystemExit("R7 deployment manifest has an unexpected token contract")

required = {"FAUCET_PG_URL", "FAUCET_ADDRESS"}
removed = {
    "FAUCET_TOKEN_ALLOCATION_WEI",
    "FAUCET_TOKEN_DAILY_CAP_WEI",
    "FAUCET_TOKEN_ADDRESS_LIFETIME_CAP_WEI",
    "FAUCET_TOKEN_HOT_WALLET_FLOAT_WEI",
}
values = {}
for line in lines:
    item = line.strip()
    if not item or item.startswith("#") or "=" not in item:
        continue
    key, value = item.split("=", 1)
    values[key] = value
missing = sorted(key for key in required if not values.get(key))
if missing:
    raise SystemExit("Protected faucet config is missing required reconciliation fields: " + ", ".join(missing))
if values.get("FAUCET_CHAIN_ID", network_config["FAUCET_CHAIN_ID"]) != network_config["FAUCET_CHAIN_ID"]:
    raise SystemExit("Protected faucet config is not configured for Sepolia")
if values.get("FAUCET_TOKEN_CONTRACT", network_config["FAUCET_TOKEN_CONTRACT"]).lower() != network_config["FAUCET_TOKEN_CONTRACT"].lower():
    raise SystemExit("Protected faucet config does not match the approved R7 Sepolia token")

pg_url = urlsplit(values["FAUCET_PG_URL"])
if pg_url.scheme not in {"postgres", "postgresql"} or not pg_url.hostname:
    raise SystemExit("FAUCET_PG_URL is not a PostgreSQL URL")
userinfo = pg_url.netloc.rpartition("@")[0]
netloc = (userinfo + "@" if userinfo else "") + db_host
if pg_url.port:
    netloc += ":" + str(pg_url.port)
candidate_pg_url = urlunsplit((pg_url.scheme, netloc, pg_url.path, pg_url.query, pg_url.fragment))

overrides = {
    **network_config,
    "FAUCET_PG_URL": candidate_pg_url,
    "FAUCET_NATIVE_DRIP_AMOUNT": "0",
}
output = []
seen = set()
for line in lines:
    item = line.strip()
    if not item or item.startswith("#") or "=" not in item:
        output.append(line)
        continue
    key, _ = item.split("=", 1)
    if key in removed:
        continue
    if key in overrides:
        if key in seen:
            continue
        output.append(key + "=" + overrides[key] + "\n")
        seen.add(key)
    else:
        output.append(line if line.endswith("\n") else line + "\n")
        seen.add(key)
for key, value in overrides.items():
    if key not in seen:
        output.append(key + "=" + value + "\n")
with open(target, "w", encoding="utf-8") as stream:
    stream.writelines(output)
os.chmod(target, 0o600)
PY

sudo docker pull "$image" >/dev/null
sudo docker volume create "$candidate_volume" >/dev/null
sudo docker run -d --name "$candidate_db" --restart unless-stopped \
  --network "$network" --network-alias "$candidate_db_alias" \
  --env-file "$candidate_dir/postgres.env" \
  --mount "type=volume,source=$candidate_volume,target=/var/lib/postgresql/data" \
  --health-cmd 'pg_isready -U "$POSTGRES_USER" -d "$POSTGRES_DB"' \
  --health-interval 10s --health-timeout 5s --health-retries 5 \
  "$live_db_image" >/dev/null

ready=false
for attempt in $(seq 1 60); do
  if sudo docker exec "$candidate_db" sh -c 'pg_isready -U "$POSTGRES_USER" -d "$POSTGRES_DB"' >/dev/null 2>&1; then
    ready=true
    break
  fi
  sleep 2
done
[[ "$ready" == true ]] || { echo 'Candidate PostgreSQL did not become ready; live service remains untouched' >&2; exit 1; }

# Stream a point-in-time logical snapshot from the live database into the
# separate candidate database. No production rows or schema are modified.
sudo docker exec "$live_db" sh -c 'pg_dump -Fc -U "$POSTGRES_USER" -d "$POSTGRES_DB"' \
  | sudo docker exec -i "$candidate_db" sh -c 'pg_restore --exit-on-error --no-owner --no-privileges -U "$POSTGRES_USER" -d "$POSTGRES_DB"'

sudo docker run -d --name "$candidate_app" --restart unless-stopped \
  --network "$network" --env-file "$candidate_dir/faucet.env" \
  -p "127.0.0.1:$candidate_port:8082" "$image" >/dev/null

ready=false
for attempt in $(seq 1 90); do
  if curl -fsS "http://127.0.0.1:$candidate_port/health" >/tmp/creg-faucet-ledger-stage-r6-health.json 2>/dev/null; then
    if python3 -c 'import json; h=json.load(open("/tmp/creg-faucet-ledger-stage-r6-health.json")); raise SystemExit(0 if h.get("ledger_reconciled") and not h.get("token_claims_enabled") else 1)' 2>/dev/null; then
      ready=true
      break
    fi
  fi
  sleep 5
done
[[ "$ready" == true ]] || { echo 'Candidate failed its reconciliation or claims-disabled health gate; inspect protected host logs. Live service remains untouched.' >&2; exit 1; }

# Check admin recovery without exposing the token or individual claim records.
recovery_summary=$(sudo docker exec -i "$candidate_app" sh -c 'printf "Authorization: Bearer %s\n" "$FAUCET_ADMIN_TOKEN" | curl -fsS -H @- http://127.0.0.1:8082/admin/recovery' \
  | python3 -c 'import json,sys; r=json.load(sys.stdin); print("status="+str(r.get("status"))+" unresolved="+str(len(r.get("claims", [])))+" claims_enabled="+str(r.get("token_claims_enabled")))')
[[ "$recovery_summary" == status=clear\ unresolved=0\ claims_enabled=False ]] || { echo "Recovery gate failed: $recovery_summary" >&2; exit 1; }

network_summary=$(curl -fsS "http://127.0.0.1:$candidate_port/api/network" \
  | python3 -c 'import json,sys; n=json.load(sys.stdin); print(str(n.get("chain_id"))+" "+str(n.get("token_contract", "")).lower())')
[[ "$network_summary" == '11155111 0xbc2a72107c6765364d0a822a88a6e6c61f30eaa6' ]] || { echo "Candidate is connected to an unexpected network: $network_summary" >&2; exit 1; }

ledger_summary=$(sudo docker exec "$candidate_db" sh -c 'psql -U "$POSTGRES_USER" -d "$POSTGRES_DB" -Atc "select reconciled_through_block||chr(124)||faucet_balance_wei||chr(124)||total_supply_wei from faucet_reconciliation_state; select count(*) from faucet_token_transfers; select count(*) from faucet_funding_ledger; select count(*) from faucet_claim_ledger where token_status in (chr(114)||chr(101)||chr(115)||chr(101)||chr(114)||chr(118)||chr(101)||chr(100),chr(115)||chr(117)||chr(98)||chr(109)||chr(105)||chr(116)||chr(116)||chr(101)||chr(100))"')
[[ -n "$ledger_summary" ]] || { echo 'Candidate reconciliation tables contain no verification data' >&2; exit 1; }

[[ "$(sudo docker inspect --format '{{.Id}}' "$live_faucet")" == "$live_faucet_id" ]]
[[ "$(sudo docker inspect --format '{{.Id}}' "$live_db")" == "$live_db_id" ]]
[[ "$(sudo docker volume inspect --format '{{.Mountpoint}}' testnet_faucet-postgres-data)" == "$live_volume_id" ]]

python3 - "$ledger_summary" "$network_summary" "$recovery_summary" <<'PY'
import json
import sys

ledger, network, recovery = sys.argv[1:]
rows = ledger.splitlines()
watermark, balance, supply = rows[0].split("|")
report = {
    "candidate": "creg-faucet-ledger-stage-r6",
    "private_endpoint": "127.0.0.1:18088",
    "network": network,
    "ledger_reconciled": True,
    "claims_enabled": False,
    "recovery": recovery,
    "reconciled_through_block": int(watermark),
    "faucet_balance_wei": balance,
    "total_supply_wei": supply,
    "canonical_transfer_rows": int(rows[1]),
    "funding_rows": int(rows[2]),
    "unresolved_reservations": int(rows[3]),
    "public_route_changed": False,
    "live_faucet_or_database_replaced": False,
}
with open("/var/lib/creg/faucet-ledger-reconciliation-r6/report.json", "w", encoding="utf-8") as stream:
    json.dump(report, stream, indent=2)
    stream.write("\n")
print(json.dumps(report, indent=2))
PY
