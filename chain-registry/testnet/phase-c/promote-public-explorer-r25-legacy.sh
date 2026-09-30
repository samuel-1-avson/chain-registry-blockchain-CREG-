#!/usr/bin/env bash
# Deploy reviewed frontend repairs against the unchanged legacy public chain.
# No R7/API/spec/default/contract cutover or signing-state mutation is performed.
set -euo pipefail
file=${1:?dedicated explorer Compose file required}
browser_evidence=${2:?passing real staged-browser report required}
network=testnet_creg-unified-net
candidate=creg-explorer-public-r25
prior=creg-explorer-public-r24-canary
preserved=$candidate-before-compose
mutation_started=false
sudo docker compose -f "$file" config --quiet
python3 - "$browser_evidence" <<'PY'
import json,sys,urllib.request
with open(sys.argv[1]) as source: evidence=json.load(source)
assert evidence['status']=='passed' and len(evidence['routes'])==14
assert evidence['expected_chain']=='creg-testnet-1' and evidence['expected_height']==0
assert evidence['sepolia_rpc_chain_id']==11155111 and not evidence['javascript_errors'] and not evidence['http_5xx']
with urllib.request.urlopen('https://api.testnet.cregnet.dev/v1/chain/stats',timeout=20) as response: stats=json.load(response)
assert stats['chain_id']=='creg-testnet-1' and stats['tip_height']==0
PY
[[ "$(sudo docker inspect "$candidate" --format '{{.Config.Image}}')" == us-central1-docker.pkg.dev/gen-lang-client-0022105784/creg-testnet/creg-web-explorer@sha256:ae0d80a7bd6f5534d4814dffdf613b66c4fee31843bfec69f713ce4319fbe122 ]] || { echo 'Wrong candidate image' >&2; exit 1; }
[[ "$(sudo docker inspect "$prior" --format '{{.Config.Image}}')" == us-central1-docker.pkg.dev/gen-lang-client-0022105784/creg-testnet/creg-web-explorer@sha256:090ba37ef9f0ee228d40d9f296d7fdafde1ef224f66f6015500c13d7f9dae6c7 ]] || { echo 'Wrong prior public image' >&2; exit 1; }
if sudo docker inspect "$preserved" >/dev/null 2>&1; then echo 'Preserved candidate exists; inspect before retry' >&2; exit 1; fi
rollback() {
  if [[ "$mutation_started" == true ]]; then
    sudo docker stop --time 10 "$candidate" >/dev/null 2>&1 || true
    sudo docker update --restart no "$candidate" >/dev/null 2>&1 || true
    sudo docker network disconnect "$network" "$candidate" >/dev/null 2>&1 || true
    sudo docker network connect --alias web-explorer "$network" "$prior" >/dev/null 2>&1 || true
    sudo docker update --restart unless-stopped "$prior" >/dev/null
    sudo docker start "$prior" >/dev/null
    echo 'Frontend guard failed; retained R24 route restored. No chain or spec changed.' >&2
  fi
}
trap rollback ERR
mutation_started=true
sudo docker stop --time 10 "$prior" >/dev/null
sudo docker update --restart no "$prior" >/dev/null
sudo docker network disconnect "$network" "$prior"
sudo docker stop --time 10 "$candidate" >/dev/null
sudo docker update --restart no "$candidate" >/dev/null
sudo docker network disconnect "$network" "$candidate"
sudo docker rename "$candidate" "$preserved"
sudo env CREG_PUBLIC_EXPLORER_NODE_UPSTREAM=creg-node:8080 docker compose -f "$file" up -d --no-build
python3 - <<'PY'
import json,time,urllib.request,urllib.error
base='https://explorer.testnet.cregnet.dev'
for attempt in range(30):
    try:
        with urllib.request.urlopen(base+'/v1/chain/stats',timeout=20) as response: stats=json.load(response)
        break
    except (urllib.error.URLError,ConnectionError,TimeoutError): time.sleep(1)
assert stats['chain_id']=='creg-testnet-1' and stats['tip_height']==0 and stats['validator_count']==2
checks={}
for entry in ['/', '/sw.js', '/registerSW.js', '/manifest.webmanifest']:
    with urllib.request.urlopen(base+entry,timeout=20) as response:
        cache=response.headers.get('Cache-Control','')
        assert 'no-store' in cache,(entry,cache)
        checks[entry]={'http_status':response.status,'cache_control':cache}
req=urllib.request.Request(base+'/rpc',data=json.dumps({'jsonrpc':'2.0','id':1,'method':'eth_chainId','params':[]}).encode(),headers={'Content-Type':'application/json','User-Agent':'creg-pipeline-verification/1.0'})
with urllib.request.urlopen(req,timeout=30) as response: rpc=json.load(response)
assert rpc.get('result')=='0xaa36a7',rpc
with urllib.request.urlopen('https://faucet.testnet.cregnet.dev/health',timeout=20) as response: faucet=json.load(response)
assert faucet['status']=='healthy' and faucet['token_drips_available']
report={'captured_at_utc':time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime()),'status':'passed','public_frontend':'r25_verified_rpc','public_chain_cutover':False,'stats':stats,'entry_points':checks,'sepolia_rpc':rpc,'faucet':faucet,'prior_preserved':True,'compose_project':'creg-public-explorer'}
with open('/tmp/creg-explorer-r25c-public.json','w') as output: json.dump(report,output,indent=2)
print(json.dumps({'status':report['status'],'frontend':report['public_frontend'],'chain_id':stats['chain_id'],'height':stats['tip_height'],'chain_cutover':False}))
PY
trap - ERR
