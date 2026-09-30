#!/usr/bin/env bash
# Promotion uses the verified real service; the prior broken container is kept.
set -euo pipefail
network=testnet_creg-unified-net
candidate=creg-faucet-public-r25
prior=creg-unified-faucet
python3 - <<'PY'
import json,urllib.request
with open('/tmp/creg-faucet-r25-real-drip.json') as source: evidence=json.load(source)
assert evidence['status']=='passed' and evidence['cooldown_after_database_and_faucet_restart']['http_status']==429
with urllib.request.urlopen('http://127.0.0.1:18082/health',timeout=20) as response: health=json.load(response)
assert health['status']=='healthy' and health['mode']=='real' and health['token_drips_available']
PY
sudo docker inspect "$prior" --format '{{.Image}} {{.State.Status}}' > /tmp/creg-faucet-prior-r25.txt
sudo docker update --restart no "$prior" >/dev/null
sudo docker stop --time 10 "$prior" >/dev/null
sudo docker network disconnect "$network" "$prior"
sudo docker network disconnect "$network" "$candidate"
sudo docker network connect --alias faucet "$network" "$candidate"
sudo docker restart --time 10 "$candidate" >/dev/null
for attempt in $(seq 1 30); do
  if curl -fsS https://faucet.testnet.cregnet.dev/health > /tmp/creg-faucet-r25-public-health.json; then break; fi
  sleep 1
done
python3 - <<'PY'
import json,time,urllib.request
def get(path):
    with urllib.request.urlopen('https://faucet.testnet.cregnet.dev'+path,timeout=20) as response:
        body=response.read(); return response.status,body
status,body=get('/'); assert status==200 and b'<html' in body.lower()
status,body=get('/health'); health=json.loads(body)
assert status==200 and health['status']=='healthy' and health['token_drips_available']
status,body=get('/api/network'); network=json.loads(body)
assert status==200 and network['chain_id']==11155111 and network['token_contract'].lower()=='0xbc2a72107c6765364d0a822a88a6e6c61f30eaa6'
status,body=get('/api/challenge'); challenge=json.loads(body); assert status==200 and challenge['difficulty']>0
report={'timestamp_utc':time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime()),'status':'public_promoted_verified','root_http_status':200,'health':health,'network':network,'real_challenge_issued':True,'prior_container_preserved':True,'chain_cutover':False}
with open('/tmp/creg-faucet-r25-public-verification.json','w') as output: json.dump(report,output,indent=2)
print(json.dumps(report))
PY
