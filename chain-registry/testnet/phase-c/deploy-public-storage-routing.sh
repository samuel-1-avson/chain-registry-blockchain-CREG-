#!/usr/bin/env bash
# Deploy only the reviewed gateway/IPFS/metrics repairs, retaining chain routing.
set -euo pipefail
incoming=${1:?reviewed Caddyfile required}
active=/home/samue/creg-hosting/chain-registry-blockchain-CREG-/chain-registry/testnet/caddy/Caddyfile.unified
base=/var/lib/creg/public-pipeline-r25
sudo install -d -m 700 "$base"
backup=$base/Caddyfile-before-storage-$(date -u +%Y%m%dT%H%M%SZ)
sudo cp "$active" "$backup"
sudo docker cp "$incoming" creg-unified-caddy:/etc/caddy/Caddyfile.pipeline-r25
sudo docker exec creg-unified-caddy caddy validate --config /etc/caddy/Caddyfile.pipeline-r25 --adapter caddyfile
before_chain=$(curl -fsS https://api.testnet.cregnet.dev/v1/public/chain/stats | python3 -c 'import json,sys; print(json.load(sys.stdin)["chain_id"])')
[[ "$before_chain" == creg-testnet-1 ]]
rollback() {
  result=$?
  if (( result != 0 )); then
    sudo cp "$backup" "$active"
    sudo docker exec creg-unified-caddy caddy reload --config /etc/caddy/Caddyfile --adapter caddyfile >/dev/null
    echo 'Gateway check failed; previous routing was restored' >&2
  fi
  exit "$result"
}
trap rollback EXIT
# Write the existing file so the running container's individual bind mount
# continues to reference the updated inode.
sudo cp "$incoming" "$active"
sudo docker exec creg-unified-caddy caddy reload --config /etc/caddy/Caddyfile --adapter caddyfile
python3 - <<'PY'
import hashlib,json,time,urllib.request
def read(url,body=None):
    request=urllib.request.Request(url,data=body,headers={'User-Agent':'CREG-pipeline-acceptance/1.0'})
    with urllib.request.urlopen(request,timeout=30) as response: return response.status,response.read()
cid='QmUrMstFUhR3dvynxTaZwRg3nNYrGVgh4P95UR2QgAeBjz'
status,content=read('https://ipfs.testnet.cregnet.dev/ipfs/'+cid)
digest=hashlib.sha256(content).hexdigest()
assert status==200 and digest=='a101155c3cbdfb1e4f98f2f83c8b5e392db6accfa606df0eba8b87a5762b0366',digest
status,version=read('https://spec.testnet.cregnet.dev/ipfs/api/v0/version',b'')
assert status==200 and json.loads(version)['Version']
status,metrics=read('https://api.testnet.cregnet.dev/metrics')
assert status==200 and b'creg_' in metrics and not metrics.startswith(b'{')
status,stats=read('https://api.testnet.cregnet.dev/v1/public/chain/stats')
stats=json.loads(stats); assert stats['chain_id']=='creg-testnet-1'
status,faucet=read('https://faucet.testnet.cregnet.dev/health'); faucet=json.loads(faucet)
assert status==200 and faucet['status']=='healthy'
report={'timestamp_utc':time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime()),'status':'passed','ipfs':{'cid':cid,'bytes':len(content),'sha256':digest,'content_gateway_http_status':200,'legacy_installer_ipfs_api_http_status':200},'public_metrics':{'http_status':200,'format':'Prometheus text','chain_id':stats['chain_id']},'faucet_healthy':True,'public_chain_changed':False}
with open('/tmp/creg-public-storage-routing-r25.json','w') as output: json.dump(report,output,indent=2)
print(json.dumps(report))
PY
