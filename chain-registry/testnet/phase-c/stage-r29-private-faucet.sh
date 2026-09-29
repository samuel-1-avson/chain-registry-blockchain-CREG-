#!/usr/bin/env bash
# Stage the real R28 image with claims open on localhost only; public R28 stays closed.
set -euo pipefail
exec 9>/var/lock/creg-public-faucet-rollout.lock
flock -n 9 || { echo 'Another faucet rollout is active' >&2; exit 1; }

image='us-central1-docker.pkg.dev/gen-lang-client-0022105784/creg-testnet/creg-faucet@sha256:d44533abf09a14a47f1b0470a8fe888b90bc699babc554d7cbcad5df38212259'
public_app=creg-faucet-public-r28
private_app=creg-faucet-r29-private
db=creg-unified-faucet-db
network=testnet_creg-unified-net
source_env=/var/lib/creg/public-faucet-r28/faucet.env
work=/var/lib/creg/public-faucet-r29
classification=/home/samue/r28-funding-classification.json

sudo test -s "$source_env"
test -s "$classification"
[[ "$(sudo docker inspect --format '{{.State.Health.Status}}' "$public_app")" == healthy ]]
[[ "$(sudo docker inspect --format '{{.State.Health.Status}}' "$db")" == healthy ]]
if sudo docker inspect "$private_app" >/dev/null 2>&1; then
  echo 'Private R29 candidate already exists; inspect it before rerunning' >&2
  exit 1
fi

curl --max-time 25 -fsS https://faucet.testnet.cregnet.dev/api/supply | python3 -c '
import json,sys
s=json.load(sys.stdin); u=10**18
assert s["status"]=="available" and s["chain_id"]==11155111
assert s["block_number"]>=11810180
assert s["faucet"]["balance_wei"]==str(100000*u)
assert s["treasury"]["balance_wei"]==str(19897590*u)
assert s["total_supply_wei"]==str(20000000*u)
assert s["token_claims_enabled"] is False
'
python3 - "$classification" <<'PY'
import json,sys
c=json.load(open(sys.argv[1],encoding="utf-8"))
assert c["status"]=="classified_after_safe_sepolia_reconciliation"
assert c["transaction_hash"]=="0x11f797c1d4514973ee88770f359ce63dcafa22c27de8996c909f12d9f713df21"
assert c["token_claims_remain_closed"] is True
PY

sudo install -d -m 700 "$work"
sudo python3 - "$source_env" "$work/faucet.env" <<'PY'
import os,secrets,sys
source,target=sys.argv[1:]
lines=open(source,encoding="utf-8").readlines()
seen=set()
with open(target,"w",encoding="utf-8") as out:
    for line in lines:
        if "=" not in line or line.lstrip().startswith("#"):
            out.write(line); continue
        key=line.split("=",1)[0].strip()
        if key in seen: continue
        seen.add(key)
        if key=="FAUCET_TOKEN_DISTRIBUTION_OPEN":
            out.write(key+"=true\n")
        elif key=="FAUCET_ADMIN_TOKEN":
            out.write(key+"="+secrets.token_urlsafe(48)+"\n")
        else:
            out.write(line)
    if "FAUCET_TOKEN_DISTRIBUTION_OPEN" not in seen:
        out.write("FAUCET_TOKEN_DISTRIBUTION_OPEN=true\n")
os.chmod(target,0o600)
PY

backup="$work/production-before-r29-private-$(date -u +%Y%m%dT%H%M%SZ).dump"
sudo docker exec "$db" sh -c 'pg_dump -Fc -U "$POSTGRES_USER" -d "$POSTGRES_DB"' | sudo tee "$backup" >/dev/null
sudo chmod 600 "$backup"
sudo test -s "$backup"

sudo docker run -d --name "$private_app" --network "$network" \
  --env-file "$work/faucet.env" -p 127.0.0.1:18093:8082 "$image" >/dev/null
ready=false
for attempt in $(seq 1 45); do
  if curl --max-time 15 -fsS http://127.0.0.1:18093/health | python3 -c '
import json,sys
h=json.load(sys.stdin)
assert h["status"]=="healthy" and h["ledger_reconciled"] is True
assert h["token_claims_enabled"] is True and h["token_distribution_open"] is True
' 2>/dev/null; then
    ready=true
    break
  fi
  sleep 5
done
[[ "$ready" == true ]] || { echo 'Private R29 candidate did not become claim-ready' >&2; exit 1; }
curl --max-time 15 -fsS http://127.0.0.1:18093/api/stats | python3 -c '
import json,sys
s=json.load(sys.stdin); u=10**18
assert s["faucet_balance"]==str(100000*u)
assert s["drip_amount"]==str(100*u)
assert s["token_allocation_wei"]==str(1000000*u)
assert s["token_daily_cap_wei"]==str(50000*u)
assert s["token_address_lifetime_cap_wei"]==str(100*u)
assert s["token_hot_wallet_float_wei"]==str(100000*u)
assert s["stats"]["unresolved_claims"]==0
'
curl --max-time 15 -fsS https://faucet.testnet.cregnet.dev/api/stats | python3 -c '
import json,sys
s=json.load(sys.stdin)
assert s["token_distribution_open"] is False and s["token_claims_enabled"] is False
'
printf 'Private R29 candidate is claim-ready at 127.0.0.1:18093; public R28 remains closed. Backup: %s\n' "$backup"
