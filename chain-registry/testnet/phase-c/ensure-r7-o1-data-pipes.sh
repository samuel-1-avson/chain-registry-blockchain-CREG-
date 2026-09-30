#!/usr/bin/env bash
# Run after every O1 container replacement, before accepting its explorer/API rollout.
set -euo pipefail
container=creg-phase-c-r7-o1
sudo docker inspect "$container" >/dev/null
for network in monitoring_default testnet_creg-unified-net; do
  sudo docker network inspect "$network" >/dev/null
  attached=$(sudo docker inspect "$container" --format '{{json .NetworkSettings.Networks}}')
  if ! grep -Fq "\"$network\"" <<< "$attached"; then
    sudo docker network connect --alias "$container" "$network" "$container"
  fi
done

python3 - <<'PY'
import json,time,urllib.request

def get(port,path):
 with urllib.request.urlopen(f'http://127.0.0.1:{port}{path}',timeout=15) as response:
  return json.load(response)

expected_network_hash='1852b9a86dd3b53e7b2de4a774723a4fe48e862f9c61d14e5917b42b21055234'
for attempt in range(30):
 try:
  node=get(18080,'/v1/public/chain/stats')
  health=get(18080,'/v1/public/health')
  history=get(18080,'/v1/public/metrics/history?range=30m')
  explorer=get(3025,'/v1/public/chain/stats')
  checks={
   'signed_r7_o1':node['chain_id']=='creg-phase-c-testnet-2026-09-26-r7' and node['spec_signature_verified'] is True and node['tip_height']==node['finalized_height'] and health['node_id']=='observer-1',
   'real_history':history['status']=='ready' and history['collector']=='prometheus' and history['chain_id']==node['chain_id'] and history['network_genesis_hash']==expected_network_hash and history['queried_series']==7 and history['sample_count']>=3,
   'private_explorer_same_tip':explorer['chain_id']==node['chain_id'] and explorer['tip_height']==node['tip_height'] and explorer['tip_hash']==node['tip_hash'],
  }
  if all(checks.values()):break
 except Exception:
  checks={}
 time.sleep(3)
else:raise SystemExit('O1 data-pipe postflight failed')

print(json.dumps({'captured_at_utc':time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime()),'status':'passed','checks':checks,'chain_id':node['chain_id'],'height':node['tip_height'],'hash':node['tip_hash'],'history_samples':history['sample_count'],'public_chain_cutover':False}))
PY
