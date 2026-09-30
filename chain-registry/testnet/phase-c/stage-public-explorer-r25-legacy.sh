#!/usr/bin/env bash
# Stage the reviewed proxy/cache repair on the existing public chain. This does
# not select R7, change contracts/spec/defaults, or waive any acceptance gate.
set -euo pipefail
image=us-central1-docker.pkg.dev/gen-lang-client-0022105784/creg-testnet/creg-web-explorer@sha256:c398d8ac46abfba2996993aadaaebccd9663523b1fd59841fa3f11705a17e7e1
container=creg-explorer-public-r25
network=testnet_creg-unified-net
if [[ "${1:-}" == --verify-only ]]; then
  [[ "$(sudo docker inspect "$container" --format '{{.Config.Image}}')" == "$image" ]] || { echo 'Wrong staged image' >&2; exit 1; }
else
  sudo docker inspect "$container" >/dev/null 2>&1 && { echo 'Candidate exists; inspect before retry' >&2; exit 1; }
  sudo docker pull "$image" >/dev/null
  sudo docker run -d --name "$container" --restart unless-stopped --network "$network" \
    -e CREG_NODE_UPSTREAM=creg-node:8080 -p 127.0.0.1:3026:80 "$image" >/dev/null
fi
sudo docker exec "$container" nginx -t
python3 - <<'PY'
import json,time,urllib.request,urllib.error
base='http://127.0.0.1:3026'
for attempt in range(30):
    try:
        with urllib.request.urlopen(base+'/v1/chain/stats',timeout=20) as response: stats=json.load(response)
        break
    except (urllib.error.URLError, ConnectionError, TimeoutError): time.sleep(1)
assert stats['chain_id']=='creg-testnet-1' and stats['tip_height']==0
assert stats['tip_hash']=='068364cf778ede5a0b1c814c59b0314f9a8598a959fa5a8e08ea78bc1b296b53'
assert stats['validator_count']==2
checks={}
for entry in ['/', '/sw.js', '/registerSW.js', '/manifest.webmanifest']:
    with urllib.request.urlopen(base+entry,timeout=20) as response:
        cache=response.headers.get('Cache-Control','')
        assert 'no-store' in cache,(entry,cache)
        checks[entry]={'http_status':response.status,'cache_control':cache}
report={'captured_at_utc':time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime()),
        'status':'staged_existing_public_chain','chain_cutover':False,'stats':stats,'entry_points':checks}
with open('/tmp/creg-explorer-r25-legacy-stage.json','w') as output: json.dump(report,output,indent=2)
print(json.dumps(report))
PY
