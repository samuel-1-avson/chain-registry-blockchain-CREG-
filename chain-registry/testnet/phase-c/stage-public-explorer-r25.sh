#!/usr/bin/env bash
set -euo pipefail
image=us-central1-docker.pkg.dev/gen-lang-client-0022105784/creg-testnet/creg-web-explorer@sha256:c398d8ac46abfba2996993aadaaebccd9663523b1fd59841fa3f11705a17e7e1
network=testnet_creg-unified-net
container=creg-explorer-public-r25-canary
sudo docker inspect "$container" >/dev/null 2>&1 && { echo 'Candidate already exists; inspect before retry' >&2; exit 1; }
sudo docker pull "$image" >/dev/null
networks=$(sudo docker inspect creg-phase-c-r7-o1 --format '{{json .NetworkSettings.Networks}}')
if ! grep -Fq '"testnet_creg-unified-net"' <<< "$networks"; then
  sudo docker network connect "$network" creg-phase-c-r7-o1
fi
sudo docker run -d --name "$container" --restart unless-stopped --network "$network" \
  -e CREG_NODE_UPSTREAM=creg-phase-c-r7-o1:8080 -p 127.0.0.1:3025:80 "$image" >/dev/null
sudo docker exec "$container" nginx -t
python3 - <<'PY'
import json,time,urllib.request,urllib.error
base='http://127.0.0.1:3025'
def get(path):
    with urllib.request.urlopen(base+path, timeout=20) as response: return json.load(response)
for attempt in range(30):
    try: stats=get('/v1/public/chain/stats'); break
    except (urllib.error.URLError, ConnectionError): time.sleep(1)
assert stats['chain_id']=='creg-phase-c-testnet-2026-09-26-r7'
assert stats['tip_height']==14 and stats['finalized_height']==14 and stats['spec_signature_verified']
history=get('/v1/public/metrics/history?range=30m')
assert history['status']=='ready' and history['queried_series']==7
assert history['chain_id']==stats['chain_id'] and history['sample_count']>0
block=get('/v1/public/blocks/14')
try:
    get('/v1/operator/runtime/config')
    raise AssertionError('Unauthenticated operator route unexpectedly succeeded')
except urllib.error.HTTPError as error: assert error.code==401, error.code
report={'timestamp_utc':time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime()),'status':'private_candidate_verified', 'public_cutover':False,'stats':stats,'history':{k:v for k,v in history.items() if k!='samples'},'operator_without_credential':401,'tip_block':block}
with open('/tmp/creg-explorer-r25-stage.json','w') as output: json.dump(report,output,indent=2)
print(json.dumps({'chain_id':stats['chain_id'],'height':stats['tip_height'],'history_samples':history['sample_count'],'public_promoted':False}))
PY
