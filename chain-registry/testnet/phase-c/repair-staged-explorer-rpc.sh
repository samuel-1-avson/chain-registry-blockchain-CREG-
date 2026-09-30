#!/usr/bin/env bash
# Replace only private frontend candidates. No public alias or node is changed.
set -euo pipefail
image=us-central1-docker.pkg.dev/gen-lang-client-0022105784/creg-testnet/creg-web-explorer@sha256:58dc13b72f1d615e5371aa80df08b3ee78dc94ecd475e71f794958adeff8dc9f
prior_image=us-central1-docker.pkg.dev/gen-lang-client-0022105784/creg-testnet/creg-web-explorer@sha256:c398d8ac46abfba2996993aadaaebccd9663523b1fd59841fa3f11705a17e7e1
network=testnet_creg-unified-net
for container in creg-explorer-public-r25 creg-explorer-public-r25-canary; do
  [[ "$(sudo docker inspect "$container" --format '{{.Config.Image}}')" == "$prior_image" ]] || { echo 'Unexpected prior image' >&2; exit 1; }
  if sudo docker inspect "$container-before-rpc-repair" >/dev/null 2>&1; then echo 'Prior preserved candidate already exists' >&2; exit 1; fi
done
sudo docker pull "$image" >/dev/null
for container in creg-explorer-public-r25 creg-explorer-public-r25-canary; do
  if [[ "$container" == creg-explorer-public-r25 ]]; then upstream=creg-node:8080; port=3026; else upstream=creg-phase-c-r7-o1:8080; port=3025; fi
  sudo docker stop --time 10 "$container" >/dev/null
  sudo docker update --restart no "$container" >/dev/null
  sudo docker rename "$container" "$container-before-rpc-repair"
  sudo docker run -d --name "$container" --restart unless-stopped --network "$network" \
    -e "CREG_NODE_UPSTREAM=$upstream" -p "127.0.0.1:$port:80" "$image" >/dev/null
done
python3 - <<'PY'
import json,time,urllib.request,urllib.error
checks=[]
for port,chain,height in [(3026,'creg-testnet-1',0),(3025,'creg-phase-c-testnet-2026-09-26-r7',14)]:
    base=f'http://127.0.0.1:{port}'
    for attempt in range(30):
        try:
            with urllib.request.urlopen(base+'/v1/chain/stats',timeout=20) as response: stats=json.load(response)
            break
        except (urllib.error.URLError,ConnectionError,TimeoutError): time.sleep(1)
    assert stats['chain_id']==chain and stats['tip_height']==height
    req=urllib.request.Request(base+'/rpc',data=json.dumps({'jsonrpc':'2.0','id':1,'method':'eth_chainId','params':[]}).encode(),headers={'Content-Type':'application/json'})
    with urllib.request.urlopen(req,timeout=30) as response: rpc=json.load(response)
    assert rpc.get('result')=='0xaa36a7',rpc
    checks.append({'port':port,'stats':stats,'rpc':rpc,'rpc_http_status':200})
report={'captured_at_utc':time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime()),'status':'passed','checks':checks,'public_promoted':False,'public_chain_cutover':False}
with open('/tmp/creg-explorer-r25b-stage.json','w') as output: json.dump(report,output,indent=2)
print(json.dumps({'status':report['status'],'checks':len(checks),'sepolia_rpc_chain_id':11155111,'public_promoted':False}))
PY
