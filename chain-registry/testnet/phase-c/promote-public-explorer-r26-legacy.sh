#!/usr/bin/env bash
# Publish the reviewed R26 frontend while keeping the public legacy chain.
set -euo pipefail
compose=${1:?pass the pinned R26 explorer Compose file}
network=testnet_creg-unified-net
prior=creg-explorer-public-r25
stage=creg-explorer-public-r26-legacy-stage
next=creg-explorer-public-r26
r25=us-central1-docker.pkg.dev/gen-lang-client-0022105784/creg-testnet/creg-web-explorer@sha256:ae0d80a7bd6f5534d4814dffdf613b66c4fee31843bfec69f713ce4319fbe122
r26=us-central1-docker.pkg.dev/gen-lang-client-0022105784/creg-testnet/creg-web-explorer@sha256:9df577975d1646289e5c1a58fd90cf48b89bfaac41f59122b02d3e7aec44d792

sudo docker compose -f "$compose" config --quiet
[[ $(sudo docker inspect "$prior" --format '{{.Config.Image}}') == "$r25" ]]
[[ $(sudo docker inspect "$stage" --format '{{.Config.Image}}') == "$r26" ]]
if sudo docker inspect "$next" >/dev/null 2>&1; then
  echo 'R26 public container already exists; inspect before retry' >&2
  exit 1
fi
python3 - <<'PY'
import json,urllib.request
def stats(url):
 with urllib.request.urlopen(url,timeout=15) as response:return json.load(response)
public=stats('https://explorer.testnet.cregnet.dev/v1/public/chain/stats')
stage=stats('http://127.0.0.1:3027/v1/public/chain/stats')
r7=stats('http://127.0.0.1:3028/v1/public/chain/stats')
history=stats('http://127.0.0.1:3028/v1/public/metrics/history?range=30m')
assert public['chain_id']==stage['chain_id']=='creg-testnet-1'
assert public['tip_height']==stage['tip_height']==0
assert r7['chain_id']=='creg-phase-c-testnet-2026-09-26-r7' and r7['tip_height']>=18
assert history['status']=='ready' and history['chain_id']==r7['chain_id'] and history['sample_count']>=3
PY

mutation_started=false
rollback() {
  if [[ $mutation_started == true ]]; then
    sudo docker stop --time 10 "$next" >/dev/null 2>&1 || true
    sudo docker update --restart no "$next" >/dev/null 2>&1 || true
    sudo docker network disconnect "$network" "$next" >/dev/null 2>&1 || true
    sudo docker network connect --alias explorer --alias web-explorer "$network" "$prior" >/dev/null 2>&1 || true
    sudo docker start "$prior" >/dev/null 2>&1 || true
    echo 'R26 promotion failed; R25 public route restored.' >&2
  fi
}
trap rollback ERR
mutation_started=true
sudo docker stop --time 10 "$stage" >/dev/null
sudo docker network disconnect "$network" "$prior"
sudo docker compose -f "$compose" up -d --no-build

python3 - <<'PY'
import json,time,urllib.request,urllib.error
base='https://explorer.testnet.cregnet.dev'
for attempt in range(20):
 try:
  with urllib.request.urlopen(base+'/v1/public/chain/stats',timeout=15) as response:stats=json.load(response)
  with urllib.request.urlopen(base+'/',timeout=15) as response:index=response.read().decode()
  assert stats['chain_id']=='creg-testnet-1' and stats['tip_height']==0
  assert 'index-BjYPFeV4.js' in index
  break
 except (urllib.error.URLError,AssertionError,TimeoutError):time.sleep(2)
else:raise SystemExit('R26 public route did not converge')
print(json.dumps({'status':'public_frontend_r26_live','public_chain':stats['chain_id'],'height':stats['tip_height'],'public_chain_cutover':False}))
PY
trap - ERR
