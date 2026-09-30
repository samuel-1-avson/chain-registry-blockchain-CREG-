#!/usr/bin/env bash
# Adopt the accepted running service into its dedicated Compose declaration.
# The same external database volume and protected env are reused; no chain
# containers or signing journals are touched. Preserve prior containers.
set -euo pipefail
file=${1:?dedicated faucet Compose file required}
stamp=$(date -u +%Y%m%dT%H%M%SZ)
network=testnet_creg-unified-net
base=/var/lib/creg/public-faucet-r25
sudo test -f "$base/faucet.env" && sudo test -f "$base/postgres.env"
sudo docker compose -f "$file" config --quiet
python3 - <<'PY'
import json,urllib.request
with urllib.request.urlopen('https://faucet.testnet.cregnet.dev/health',timeout=20) as response:
    health=json.load(response)
assert health['status']=='healthy' and health['token_drips_available']
PY
for container in creg-faucet-public-r25 creg-unified-faucet-db; do
  sudo docker stop --time 10 "$container" >/dev/null
  sudo docker update --restart no "$container" >/dev/null
  sudo docker network disconnect "$network" "$container"
  sudo docker rename "$container" "$container-before-compose-$stamp"
done
sudo docker compose -f "$file" up -d --no-build
for attempt in $(seq 1 30); do
  if curl -fsS https://faucet.testnet.cregnet.dev/health > /tmp/creg-faucet-r25-compose-health.json; then break; fi
  sleep 1
done
python3 - <<'PY'
import json,subprocess,urllib.request
with urllib.request.urlopen('https://faucet.testnet.cregnet.dev/health',timeout=20) as response: health=json.load(response)
assert health['status']=='healthy' and health['token_drips_available']
records={}
for name in ['creg-faucet-public-r25','creg-unified-faucet-db']:
    meta=json.loads(subprocess.check_output(['sudo','docker','inspect',name],text=True))[0]
    labels=meta['Config']['Labels']; assert labels['com.docker.compose.project']=='creg-public-faucet'
    records[name]={'image':meta['Config']['Image'],'state':meta['State']['Status'],'compose_project':labels['com.docker.compose.project'],'restart_policy':meta['HostConfig']['RestartPolicy']['Name']}
count=subprocess.check_output(['sudo','docker','exec','creg-unified-faucet-db','psql','-U','faucet','-d','creg_faucet','-Atc',"SELECT count(*) FROM faucet_rate_limits WHERE key LIKE 'addr:%'"],text=True).strip()
assert int(count)>=1,'Accepted cooldown data was lost'
report={'status':'passed','health':health,'durable_address_count':int(count),'services':records,'other_services_recreated':False}
with open('/tmp/creg-faucet-r25-compose-adoption.json','w') as output: json.dump(report,output,indent=2)
print(json.dumps(report))
PY
