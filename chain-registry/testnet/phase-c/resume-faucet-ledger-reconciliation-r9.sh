#!/usr/bin/env bash
# Resume the private R9 candidate from its preserved PostgreSQL snapshot with
# a repaired image. This reuses only the isolated candidate DB and volume.
set -euo pipefail
exec 9>/var/lock/creg-faucet-ledger-stage.lock
flock -n 9 || { echo 'Another faucet ledger staging run is active' >&2; exit 1; }

image=${CREG_FAUCET_CANDIDATE_IMAGE:?set CREG_FAUCET_CANDIDATE_IMAGE to a digest-pinned image}
[[ "$image" == *@sha256:* ]] || { echo 'Candidate image must be pinned by sha256 digest' >&2; exit 1; }

live_faucet=creg-faucet-public-r25
live_db=creg-unified-faucet-db
network=testnet_creg-unified-net
candidate_dir=/var/lib/creg/faucet-ledger-reconciliation-r9
candidate_db=creg-faucet-ledger-stage-db-r9
candidate_app=creg-faucet-ledger-stage-r9
candidate_port=18091
status_file=/tmp/creg-faucet-ledger-stage-r9-status.json
report_dir=/var/lib/creg/faucet-ledger-reconciliation-r9

for path in "$candidate_dir/faucet.env" "$candidate_dir/postgres.env"; do
  sudo test -s "$path" || { echo "Preserved candidate config is unavailable: $path" >&2; exit 1; }
done
sudo docker inspect "$live_faucet" >/dev/null
sudo docker inspect "$live_db" >/dev/null
sudo docker inspect "$candidate_db" >/dev/null
sudo docker volume inspect testnet_faucet-ledger-stage-r9-data >/dev/null
sudo docker network inspect "$network" >/dev/null
[[ "$(sudo docker inspect --format '{{.State.Running}}' "$live_faucet")" == false ]] || { echo 'Public faucet must remain stopped during recovery validation' >&2; exit 1; }
[[ "$(sudo docker inspect --format '{{.State.Health.Status}}' "$candidate_db")" == healthy ]] || { echo 'Preserved candidate database is not healthy' >&2; exit 1; }

live_faucet_id=$(sudo docker inspect --format '{{.Id}}' "$live_faucet")
live_db_id=$(sudo docker inspect --format '{{.Id}}' "$live_db")
live_volume_id=$(sudo docker volume inspect --format '{{.Mountpoint}}' testnet_faucet-postgres-data)
live_db_image=$(sudo docker inspect --format '{{.Image}}' "$live_db")
candidate_db_id=$(sudo docker inspect --format '{{.Id}}' "$candidate_db")
candidate_db_image=$(sudo docker inspect --format '{{.Image}}' "$candidate_db")
[[ "$candidate_db_image" == "$live_db_image" ]] || { echo 'Candidate DB image differs from production DB image' >&2; exit 1; }

if sudo docker inspect "$candidate_app" >/dev/null 2>&1; then
  sudo docker stop "$candidate_app" >/dev/null || true
  sudo docker rm "$candidate_app" >/dev/null
fi
sudo docker pull "$image" >/dev/null
sudo docker run -d --name "$candidate_app" --restart unless-stopped \
  --network "$network" --env-file "$candidate_dir/faucet.env" \
  -p "127.0.0.1:$candidate_port:8082" "$image" >/dev/null

candidate_admin_status() {
  sudo docker exec -i "$candidate_app" sh -c 'printf "Authorization: Bearer %s\n" "$FAUCET_ADMIN_TOKEN" | curl --max-time 5 -sS -H @- http://127.0.0.1:8082/admin/status' >"$status_file" 2>/dev/null
}

await_reconciled() {
  local ready=false
  for attempt in $(seq 1 90); do
    if candidate_admin_status && python3 -c 'import json; s=json.load(open("/tmp/creg-faucet-ledger-stage-r9-status.json")); raise SystemExit(0 if s.get("ledger_reconciled") and not s.get("token_claims_enabled") else 1)' 2>/dev/null; then
      ready=true
      break
    fi
    sleep 5
  done
  [[ "$ready" == true ]]
}

await_reconciled || { echo 'Candidate did not reconcile the preserved ledger; claims remain disabled' >&2; exit 1; }
sudo docker restart "$candidate_app" >/dev/null
await_reconciled || { echo 'Candidate did not recover its ledger after restart; claims remain disabled' >&2; exit 1; }

recovery_summary=$(sudo docker exec -i "$candidate_app" sh -c 'printf "Authorization: Bearer %s\n" "$FAUCET_ADMIN_TOKEN" | curl --max-time 5 -fsS -H @- http://127.0.0.1:8082/admin/recovery' \
  | python3 -c 'import json,sys; r=json.load(sys.stdin); print("status="+str(r.get("status"))+" unresolved="+str(len(r.get("claims", [])))+" claims_enabled="+str(r.get("token_claims_enabled")))')
[[ "$recovery_summary" == status=clear\ unresolved=0\ claims_enabled=False ]] || { echo "Recovery gate failed: $recovery_summary" >&2; exit 1; }

network_summary=$(curl --max-time 5 -fsS "http://127.0.0.1:$candidate_port/api/network" \
  | python3 -c 'import json,sys; n=json.load(sys.stdin); print(str(n.get("chain_id"))+" "+str(n.get("token_contract", "")).lower())')
[[ "$network_summary" == '11155111 0xbc2a72107c6765364d0a822a88a6e6c61f30eaa6' ]] || { echo "Candidate is connected to an unexpected network: $network_summary" >&2; exit 1; }

ledger_summary=$(sudo docker exec "$candidate_db" sh -c 'psql -U "$POSTGRES_USER" -d "$POSTGRES_DB" -Atc "select reconciled_through_block||chr(124)||faucet_balance_wei||chr(124)||total_supply_wei from faucet_reconciliation_state; select count(*) from faucet_token_transfers; select count(*) from faucet_funding_ledger; select count(*) from faucet_claim_ledger where token_status in (chr(114)||chr(101)||chr(115)||chr(101)||chr(114)||chr(118)||chr(101)||chr(100),chr(115)||chr(117)||chr(98)||chr(109)||chr(105)||chr(116)||chr(116)||chr(101)||chr(100)) or native_status in (chr(114)||chr(101)||chr(115)||chr(101)||chr(114)||chr(118)||chr(101)||chr(100),chr(115)||chr(117)||chr(98)||chr(109)||chr(105)||chr(116)||chr(116)||chr(101)||chr(100))"')
[[ -n "$ledger_summary" ]] || { echo 'Candidate reconciliation tables contain no verification data' >&2; exit 1; }

[[ "$(sudo docker inspect --format '{{.Id}}' "$candidate_db")" == "$candidate_db_id" ]]
[[ "$(sudo docker inspect --format '{{.Id}}' "$live_faucet")" == "$live_faucet_id" ]]
[[ "$(sudo docker inspect --format '{{.Id}}' "$live_db")" == "$live_db_id" ]]
[[ "$(sudo docker volume inspect --format '{{.Mountpoint}}' testnet_faucet-postgres-data)" == "$live_volume_id" ]]

python3 - "$ledger_summary" "$network_summary" "$recovery_summary" "$image" <<'PY'
import json
import sys

ledger, network, recovery, image = sys.argv[1:]
rows = ledger.splitlines()
watermark, balance, supply = rows[0].split("|")
report = {
    "candidate": "creg-faucet-ledger-stage-r9",
    "private_endpoint": "127.0.0.1:18091",
    "image": image,
    "network": network,
    "ledger_reconciled": True,
    "claims_enabled": False,
    "restart_recovery_confirmed": True,
    "recovery": recovery,
    "reconciled_through_block": int(watermark),
    "faucet_balance_wei": balance,
    "total_supply_wei": supply,
    "canonical_transfer_rows": int(rows[1]),
    "funding_rows": int(rows[2]),
    "unresolved_reservations": int(rows[3]),
    "public_route_changed": False,
    "live_faucet_or_database_replaced": False,
    "production_database_volume_reused": False,
}
with open("/var/lib/creg/faucet-ledger-reconciliation-r9/recovery-report.json", "w", encoding="utf-8") as stream:
    json.dump(report, stream, indent=2)
    stream.write("\n")
print(json.dumps(report, indent=2))
PY
