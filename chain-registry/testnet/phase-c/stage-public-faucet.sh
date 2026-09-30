#!/usr/bin/env bash
# Stage the real, funded Sepolia faucet and durable cooldown database privately.
set -euo pipefail
incoming=${1:?protected incoming directory required}
base=/var/lib/creg/public-faucet-r25
network=testnet_creg-unified-net
image=${CREG_FAUCET_IMAGE:?set CREG_FAUCET_IMAGE to the immutable image built with faucet reconciliation}
[[ "$image" == *@sha256:* ]] || { echo 'CREG_FAUCET_IMAGE must be pinned by sha256 digest' >&2; exit 1; }
for name in FAUCET_TOKEN_CONTRACT FAUCET_TOKEN_DEPLOYMENT_TX_HASH FAUCET_TOKEN_DEPLOYMENT_BLOCK FAUCET_RECONCILIATION_RPC_URL FAUCET_DEPLOYMENT_RECEIPT_RPC_URL; do
  grep -Eq "^${name}=.+$" "$incoming/faucet.env" || { echo "Missing $name in protected faucet.env" >&2; exit 1; }
done
sudo install -d -m 700 "$base"
for name in faucet.env postgres.env; do
  sudo test ! -e "$base/$name" || { echo 'Protected configuration already exists; inspect before retry' >&2; exit 1; }
  sudo install -m 600 "$incoming/$name" "$base/$name"
done
sudo docker inspect creg-faucet-public-r25 >/dev/null 2>&1 && { echo 'Faucet candidate already exists' >&2; exit 1; }
sudo docker pull postgres:16-alpine >/dev/null
postgres_image=$(sudo docker image inspect postgres:16-alpine --format '{{index .RepoDigests 0}}')
[[ "$postgres_image" == postgres@sha256:* ]]
sudo docker volume create testnet_faucet-postgres-data >/dev/null
sudo docker run -d --name creg-unified-faucet-db --restart unless-stopped \
  --network "$network" --network-alias faucet-db --env-file "$base/postgres.env" \
  --mount type=volume,source=testnet_faucet-postgres-data,target=/var/lib/postgresql/data \
  --health-cmd 'pg_isready -U faucet -d creg_faucet' --health-interval 10s --health-timeout 5s --health-retries 5 \
  "$postgres_image" >/dev/null
for attempt in $(seq 1 30); do
  if sudo docker exec creg-unified-faucet-db pg_isready -U faucet -d creg_faucet >/dev/null 2>&1; then break; fi
  sleep 1
done
sudo docker exec creg-unified-faucet-db pg_isready -U faucet -d creg_faucet >/dev/null
sudo docker run -d --name creg-faucet-public-r25 --restart unless-stopped \
  --network "$network" --env-file "$base/faucet.env" \
  -p 127.0.0.1:18082:8082 "$image" >/dev/null
for attempt in $(seq 1 30); do
  if curl -fsS http://127.0.0.1:18082/health > /tmp/creg-faucet-r25-health.json; then break; fi
  sleep 1
done
python3 - <<'PY'
import json, urllib.request
def get(path):
    with urllib.request.urlopen('http://127.0.0.1:18082'+path, timeout=20) as response:
        return json.load(response)
health=get('/health'); network=get('/api/network'); challenge=get('/api/challenge')
assert health['mode']=='real' and health['ledger_reconciled'], health
assert not health['token_claims_enabled'] and not health['token_drips_available'], health
assert network['chain_id']==11155111, network
assert network['token_contract'].lower()=='0xbc2a72107c6765364d0a822a88a6e6c61f30eaa6', network
assert challenge['difficulty']>0 and len(challenge['challenge'])>0, challenge
report={'health':health,'network':network,'proof_of_work_challenge':'real challenge issued', 'token_claims_enabled':False, 'public_promoted':False}
with open('/tmp/creg-faucet-r25-stage.json','w') as output: json.dump(report,output,indent=2)
print(json.dumps(report))
PY
echo "PostgreSQL image: $postgres_image"
