#!/usr/bin/env bash
# Use the fresh Sepolia proxy for finalized recent history and Routescan for
# the pruned deployment receipt; keep claims closed throughout.
set -euo pipefail
exec 9>/var/lock/creg-public-faucet-rollout.lock
flock -n 9 || { echo 'Another faucet rollout is active' >&2; exit 1; }

app=creg-faucet-public-r28
db=creg-unified-faucet-db
compose=/home/samue/docker-compose.public-faucet.yml
env_file=/var/lib/creg/public-faucet-r28/faucet.env
work=/var/lib/creg/public-faucet-r28
backup="$work/faucet.env.before-history-source-$(date -u +%Y%m%dT%H%M%SZ)"
report="$work/history-source-repair.json"
switched=false

rollback() {
  status=$?
  if [[ $status -ne 0 && "$switched" == true ]]; then
    echo 'Fresh history source failed; restoring prior closed-claims configuration' >&2
    sudo cp -p "$backup" "$env_file"
    sudo docker compose -f "$compose" up -d --force-recreate --no-deps faucet || true
  fi
}
trap rollback EXIT

sudo test -s "$env_file"
[[ "$(sudo docker inspect --format '{{.State.Health.Status}}' "$app")" == healthy ]]
[[ "$(sudo docker inspect --format '{{.State.Health.Status}}' "$db")" == healthy ]]
db_id=$(sudo docker inspect --format '{{.Id}}' "$db")
volume_mount=$(sudo docker volume inspect --format '{{.Mountpoint}}' testnet_faucet-postgres-data)
sudo cp -p "$env_file" "$backup"

sudo python3 - "$env_file" <<'PY'
import os,sys
path=sys.argv[1]
values={
 "FAUCET_RECONCILIATION_RPC_URL":"https://api.testnet.cregnet.dev/sepolia",
 "FAUCET_RECONCILIATION_INDEXER_URL":"",
 "FAUCET_DEPLOYMENT_RECEIPT_RPC_URL":"https://api.routescan.io/v2/network/testnet/evm/11155111/etherscan/api",
 "FAUCET_TOKEN_DISTRIBUTION_OPEN":"false",
}
lines=open(path,encoding="utf-8").readlines()
seen=set()
with open(path,"w",encoding="utf-8") as stream:
  for line in lines:
    if "=" not in line or line.lstrip().startswith("#"):
      stream.write(line); continue
    name=line.split("=",1)[0].strip()
    if name in seen: continue
    seen.add(name)
    stream.write(name+"="+values[name]+"\n" if name in values else line)
  for name,value in values.items():
    if name not in seen:
      stream.write(name+"="+value+"\n")
os.chmod(path,0o600)
PY
switched=true
sudo docker compose -f "$compose" up -d --force-recreate --no-deps faucet >/dev/null
[[ "$(sudo docker inspect --format '{{.Id}}' "$db")" == "$db_id" ]]
[[ "$(sudo docker volume inspect --format '{{.Mountpoint}}' testnet_faucet-postgres-data)" == "$volume_mount" ]]

ready=false
for attempt in $(seq 1 60); do
  if curl --max-time 20 -fsS https://faucet.testnet.cregnet.dev/api/supply | python3 -c '
import json,sys
s=json.load(sys.stdin);u=10**18
assert s["status"]=="available" and s["chain_id"]==11155111
assert s["block_number"]>=11810180
assert s["faucet"]["balance_wei"]==str(100000*u)
assert s["treasury"]["balance_wei"]==str(19897590*u)
assert s["total_supply_wei"]==str(20000000*u)
assert s["token_claims_enabled"] is False
' 2>/dev/null; then
    ready=true
    break
  fi
  sleep 5
done
[[ "$ready" == true ]] || { echo 'Safe public faucet history did not reach the funding transfer' >&2; exit 1; }
curl --max-time 15 -fsS https://faucet.testnet.cregnet.dev/api/stats | python3 -c '
import json,sys
s=json.load(sys.stdin)
assert s["status"]=="available" and s["ledger_reconciled"] is True
assert s["token_claims_enabled"] is False and s["token_distribution_open"] is False
assert s["faucet_balance"]==str(100000*10**18)
'
[[ "$(sudo docker inspect --format '{{.State.Health.Status}}' "$app")" == healthy ]]

sudo python3 - "$report" <<'PY'
import json,sys,urllib.request
from datetime import datetime,timezone
base="https://faucet.testnet.cregnet.dev"
def read(path):
  with urllib.request.urlopen(base+path,timeout=20) as response:
    return json.load(response)
report={
 "verified_at_utc":datetime.now(timezone.utc).isoformat(),
 "status":"safe_history_advanced_with_fresh_finalized_rpc",
 "recent_history_source":"CREG Sepolia proxy",
 "deployment_receipt_source":"Routescan archive",
 "claims_remain_closed":True,
 "supply":read("/api/supply"),
 "stats":read("/api/stats"),
 "on_chain_transaction_sent":False,
}
with open(sys.argv[1],"w",encoding="utf-8") as stream:
  json.dump(report,stream,indent=2);stream.write("\n")
print(json.dumps({"status":report["status"],"safe_block":report["supply"]["block_number"],
                  "faucet_balance_wei":report["supply"]["faucet"]["balance_wei"]}))
PY
switched=false
