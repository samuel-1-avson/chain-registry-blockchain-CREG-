#!/usr/bin/env bash
set -euo pipefail
archive=/var/tmp/creg-explorer-pipelines-20260926.tar.gz
release=/var/lib/creg/explorer-releases/pipelines-20260926
image=chain-registry-web-explorer:pipelines-20260926
network=testnet_creg-unified-net
old_image=sha256:687c53d9a97c4d6debb1c817cd73136ccd3ff5e1d32124cbdd881e8fdab2b246
backup=creg-explorer-before-pipelines-20260926
compose=/home/samue/creg-hosting/chain-registry-blockchain-CREG-/chain-registry/testnet/docker-compose.unified.yml
echo '25c9278a070f50ed04ec75f64317bc81647d260ccd85048dadf13158bd02199d  /var/tmp/creg-explorer-pipelines-20260926.tar.gz' | sha256sum -c -
test "$(docker inspect creg-unified-explorer --format '{{.Image}}')" = "$old_image"
test "$(docker image inspect chain-registry-web-explorer:3node --format '{{.Id}}')" = "$old_image"
test -f "$compose"
test ! -e "$release"
if docker inspect "$backup" >/dev/null 2>&1 || docker inspect creg-explorer-pipelines-review >/dev/null 2>&1; then
  echo 'Release/backup container already exists; inspect before continuing.' >&2; exit 1
fi
mkdir -p "$release"
tar -xzf "$archive" -C "$release"
docker build --pull=false -t "$image" "$release"
docker run -d --name creg-explorer-pipelines-review --network "$network" -p 127.0.0.1:18090:80 "$image"
docker exec creg-explorer-pipelines-review nginx -t
python3 - <<'PY'
import json,time,urllib.request
for attempt in range(10):
    try:
        with urllib.request.urlopen('http://127.0.0.1:18090/v1/public/chain/stats',timeout=10) as response:
            stats=json.load(response)
        assert stats['chain_id']=='creg-testnet-1'
        assert stats['genesis_hash']=='068364cf778ede5a0b1c814c59b0314f9a8598a959fa5a8e08ea78bc1b296b53'
        print(json.dumps({'preflight':True,'chain_id':stats['chain_id'],'height':stats['tip_height']}))
        break
    except Exception:
        if attempt==9: raise
        time.sleep(1)
PY
switched=false
rollback() {
  if [ "$switched" = true ]; then
    if docker inspect creg-unified-explorer >/dev/null 2>&1; then
      test "$(docker inspect creg-unified-explorer --format '{{.Config.Image}}')" = "$image" || exit 1
      docker rm -f creg-unified-explorer
    fi
    docker rename "$backup" creg-unified-explorer
    docker start creg-unified-explorer
  fi
}
trap rollback ERR
docker stop creg-unified-explorer
docker rename creg-unified-explorer "$backup"
switched=true
docker run -d --name creg-unified-explorer --restart unless-stopped --network "$network" --network-alias web-explorer "$image"
docker exec creg-unified-explorer nginx -t
docker exec creg-unified-explorer curl -fsS http://127.0.0.1/health
python3 - <<'PY'
import pathlib,shutil
p=pathlib.Path('/home/samue/creg-hosting/chain-registry-blockchain-CREG-/chain-registry/testnet/docker-compose.unified.yml')
s=p.read_text()
old='image: chain-registry-web-explorer:3node'
assert s.count(old)==1,'Unexpected explorer deployment configuration'
shutil.copy2(p,str(p)+'.before-pipelines-20260926')
p.write_text(s.replace(old,'image: chain-registry-web-explorer:pipelines-20260926'))
PY
trap - ERR
docker rm -f creg-explorer-pipelines-review
docker inspect creg-unified-explorer --format '{{.Image}} {{.State.Status}}'
echo 'Web release applied. Public node and default chain spec unchanged.'
