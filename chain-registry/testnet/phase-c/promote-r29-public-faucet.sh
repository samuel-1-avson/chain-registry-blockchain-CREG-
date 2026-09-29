#!/usr/bin/env bash
# Promote the receipt-backed private claim candidate after safe Sepolia import.
set -euo pipefail
exec 9>/var/lock/creg-public-faucet-rollout.lock
flock -n 9 || { echo 'Another faucet rollout is active' >&2; exit 1; }

work=/var/lib/creg/public-faucet-r29
db=creg-unified-faucet-db
public_old=creg-faucet-public-r28
private_app=creg-faucet-r29-private
public_new=creg-faucet-public-r29
compose=/home/samue/docker-compose.public-faucet.yml
candidate_compose=/home/samue/docker-compose.public-faucet-r29.yml
claim=/home/samue/r29-real-claim.json
backup_compose="$work/public-compose-r28.yml"
supply_file=$(mktemp /tmp/creg-r29-public-supply.XXXXXX)
stats_file=$(mktemp /tmp/creg-r29-public-stats.XXXXXX)
health_file=$(mktemp /tmp/creg-r29-public-health.XXXXXX)
switched=false

cleanup() {
  status=$?
  rm -f "$supply_file" "$stats_file" "$health_file"
  if [[ $status -ne 0 && "$switched" == true ]]; then
    echo 'R29 public verification failed; restoring closed R28 Compose' >&2
    sudo cp "$backup_compose" "$compose"
    sudo docker compose -f "$compose" up -d --no-deps faucet || true
  fi
}
trap cleanup EXIT

sudo test -s "$work/faucet.env"
test -s "$candidate_compose"
test -s "$claim"
[[ "$(sudo docker inspect --format '{{.State.Health.Status}}' "$db")" == healthy ]]
[[ "$(sudo docker inspect --format '{{.State.Health.Status}}' "$public_old")" == healthy ]]
[[ "$(sudo docker inspect --format '{{.State.Health.Status}}' "$private_app")" == healthy ]]
sudo docker compose -f "$candidate_compose" config --quiet

python3 - "$claim" <<'PY'
import json,sys
c=json.load(open(sys.argv[1],encoding="utf-8"))
assert c["status"]=="passed" and c["private_candidate_restart"]=="passed"
assert c["expected_claim_wei"]==str(100*10**18)
assert c["recipient_after_wei"]==str(100*10**18)
assert c["faucet_after_wei"]==str(99900*10**18)
assert c["public_claims_open"] is False
assert c["requests_mocked"] is False
PY

curl --max-time 25 -fsS https://faucet.testnet.cregnet.dev/api/supply >"$supply_file"
python3 - "$supply_file" "$claim" <<'PY'
import json,sys
s=json.load(open(sys.argv[1],encoding="utf-8"))
c=json.load(open(sys.argv[2],encoding="utf-8"))
u=10**18
assert s["status"]=="available" and s["chain_id"]==11155111
assert s["block_number"]>=c["receipt_block_number"]
assert s["faucet"]["balance_wei"]==str(99900*u)
assert s["treasury"]["balance_wei"]==str(19897590*u)
assert s["total_supply_wei"]==str(20000000*u)
assert s["token_claims_enabled"] is False
PY
curl --max-time 15 -fsS http://127.0.0.1:18093/api/stats >"$stats_file"
python3 - "$stats_file" <<'PY'
import json,sys
s=json.load(open(sys.argv[1],encoding="utf-8")); u=10**18
assert s["token_claims_enabled"] is True and s["token_distribution_open"] is True
assert s["ledger_reconciled"] is True and s["token_claim_policy_ready"] is True
assert s["faucet_balance"]==str(99900*u)
assert s["stats"]["total_distributed"]==str(2100*u)
assert s["stats"]["unresolved_claims"]==0
PY

db_id=$(sudo docker inspect --format '{{.Id}}' "$db")
volume_mount=$(sudo docker volume inspect --format '{{.Mountpoint}}' testnet_faucet-postgres-data)
sudo cp -p "$compose" "$backup_compose"
db_backup="$work/production-before-r29-public-$(date -u +%Y%m%dT%H%M%SZ).dump"
sudo docker exec "$db" sh -c 'pg_dump -Fc -U "$POSTGRES_USER" -d "$POSTGRES_DB"' | sudo tee "$db_backup" >/dev/null
sudo chmod 600 "$db_backup"
sudo test -s "$db_backup"

sudo docker stop "$private_app" >/dev/null
sudo docker rm "$private_app" >/dev/null
sudo cp "$candidate_compose" "$compose"
switched=true
sudo docker compose -f "$compose" up -d --no-deps faucet >/dev/null
[[ "$(sudo docker inspect --format '{{.Id}}' "$db")" == "$db_id" ]]
[[ "$(sudo docker volume inspect --format '{{.Mountpoint}}' testnet_faucet-postgres-data)" == "$volume_mount" ]]

ready=false
for attempt in $(seq 1 45); do
  if curl --max-time 15 -fsS https://faucet.testnet.cregnet.dev/api/stats >"$stats_file" &&
     python3 - "$stats_file" 2>/dev/null <<'PY' &&
import json,sys
s=json.load(open(sys.argv[1],encoding="utf-8")); u=10**18
assert s["status"]=="available" and s["ledger_reconciled"] is True
assert s["token_claims_enabled"] is True and s["token_distribution_open"] is True
assert s["token_claim_policy_ready"] is True
assert s["faucet_balance"]==str(99900*u)
assert s["drip_amount"]==str(100*u)
assert s["token_allocation_wei"]==str(1000000*u)
assert s["token_daily_cap_wei"]==str(50000*u)
assert s["token_address_lifetime_cap_wei"]==str(100*u)
assert s["token_hot_wallet_float_wei"]==str(100000*u)
assert s["stats"]["total_distributed"]==str(2100*u)
assert s["stats"]["unresolved_claims"]==0
PY
     curl --max-time 15 -fsS https://faucet.testnet.cregnet.dev/health >"$health_file" &&
     python3 - "$health_file" 2>/dev/null <<'PY' &&
import json,sys
h=json.load(open(sys.argv[1],encoding="utf-8"))
assert h["status"]=="healthy" and h["ledger_reconciled"] is True
assert h["token_claims_enabled"] is True and h["token_distribution_open"] is True
PY
     curl --max-time 25 -fsS https://faucet.testnet.cregnet.dev/api/supply >"$supply_file" &&
     python3 - "$supply_file" "$claim" 2>/dev/null <<'PY'
import json,sys
s=json.load(open(sys.argv[1],encoding="utf-8"))
c=json.load(open(sys.argv[2],encoding="utf-8"))
u=10**18
assert s["status"]=="available" and s["chain_id"]==11155111
assert s["block_number"]>=c["receipt_block_number"]
assert s["faucet"]["balance_wei"]==str(99900*u)
assert s["treasury"]["balance_wei"]==str(19897590*u)
assert s["total_supply_wei"]==str(20000000*u)
assert s["token_claims_enabled"] is True
PY
  then
    ready=true
    break
  fi
  sleep 3
done
[[ "$ready" == true ]] || { echo 'Public R29 did not pass live release gate' >&2; exit 1; }
[[ "$(sudo docker inspect --format '{{.State.Health.Status}}' "$public_new")" == healthy ]]

sudo python3 - "$work/report.json" "$db_backup" "$supply_file" "$stats_file" "$claim" <<'PY'
import json,sys
from datetime import datetime,timezone
out,db_backup,supply_path,stats_path,claim_path=sys.argv[1:]
report={
  "verified_at_utc":datetime.now(timezone.utc).isoformat(),
  "status":"public_r29_open_after_real_claim_and_safe_import",
  "public_url":"https://faucet.testnet.cregnet.dev/",
  "production_database_container_and_volume_preserved":True,
  "production_database_backup":db_backup,
  "claim":json.load(open(claim_path,encoding="utf-8")),
  "supply":json.load(open(supply_path,encoding="utf-8")),
  "stats":json.load(open(stats_path,encoding="utf-8")),
  "requests_mocked":False,
  "new_on_chain_transaction_sent_by_promotion":False,
}
with open(out,"w",encoding="utf-8") as stream:
  json.dump(report,stream,indent=2)
  stream.write("\n")
print(json.dumps({"status":report["status"],"claim_tx":report["claim"]["token_tx_hash"],
                  "safe_block":report["supply"]["block_number"],
                  "faucet_balance_wei":report["stats"]["faucet_balance"]}))
PY
switched=false
