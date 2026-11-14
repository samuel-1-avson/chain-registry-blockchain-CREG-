#!/usr/bin/env bash
set -euo pipefail

archive=/var/tmp/web-release-r14.tar.gz
release=/var/lib/creg/explorer-releases/pipelines-r14-20260926
image=chain-registry-web-explorer:pipelines-r14-20260926
base_image=chain-registry-web-explorer:pipelines-r13-20260926
network=testnet_creg-unified-net
old_image=sha256:d2bcef4f75806f18edd8d189b8c059eaaa89ea9e2669f3bedfa5f6cb9af657da
backup=creg-explorer-before-pipelines-r14-20260926
review=creg-explorer-pipelines-review-r14
compose=/home/samue/creg-hosting/chain-registry-blockchain-CREG-/chain-registry/testnet/docker-compose.unified.yml
compose_backup=${compose}.before-pipelines-r14-20260926

echo '0d3a1d11f0a82ec2f76844d567c6b073174d93f2e81d6ef8bf2ea56dd1bef3af  /var/tmp/web-release-r14.tar.gz' | sha256sum -c -
test "$(sudo -n docker inspect creg-unified-explorer --format '{{.Image}}')" = "$old_image"
test "$(sudo -n docker image inspect "$base_image" --format '{{.Id}}')" = "$old_image"
test -f "$compose"
grep -Fq 'image: chain-registry-web-explorer:pipelines-r13-20260926' "$compose"
test ! -e "$compose_backup"
if sudo -n docker inspect "$backup" >/dev/null 2>&1 || sudo -n docker inspect "$review" >/dev/null 2>&1; then
  echo 'Release, backup, or review container already exists; inspect before continuing.' >&2
  exit 1
fi

sudo -n mkdir -p "$release"
sudo -n tar -xzf "$archive" -C "$release"
sudo -n docker build --pull=false -t "$image" "$release"
cleanup_review() {
  status=$?
  if sudo -n docker inspect "$review" >/dev/null 2>&1; then sudo -n docker rm -f "$review"; fi
  exit "$status"
}
trap cleanup_review ERR
sudo -n docker run -d --name "$review" --network "$network" -p 127.0.0.1:18091:80 "$image"
sudo -n docker exec "$review" nginx -t
python3 - <<'PY'
import json, time, urllib.request

base = 'http://127.0.0.1:18091'
def get(path):
    with urllib.request.urlopen(base + path, timeout=12) as response:
        body = response.read()
        return response.status, response.headers.get('Content-Type', ''), body

for attempt in range(12):
    try:
        status, content_type, payload = get('/v1/public/chain/stats')
        stats = json.loads(payload)
        assert status == 200 and stats['chain_id'] == 'creg-testnet-1'
        assert stats['tip_height'] == 0
        assert stats['genesis_hash'] == '068364cf778ede5a0b1c814c59b0314f9a8598a959fa5a8e08ea78bc1b296b53'
        assert stats['spec_signature_verified'] is True

        status, content_type, payload = get('/chain-spec.json')
        public_spec = json.loads(payload)
        assert status == 200 and 'application/json' in content_type
        assert public_spec['chain_id'] == 'creg-testnet-1'

        status, content_type, payload = get('/phase-c/r7/chain-spec.json')
        candidate_spec = json.loads(payload)
        assert status == 200 and 'application/json' in content_type
        assert candidate_spec['chain_id'] == 'creg-phase-c-testnet-2026-09-26-r7'

        status, content_type, payload = get('/chain-spec.json.sig')
        assert status == 200 and len(payload.strip()) >= 80

        status, content_type, payload = get('/api-docs/')
        assert status == 200 and b'Swagger UI' in payload

        print(json.dumps({'preflight': True, 'public_chain_id': stats['chain_id'],
                          'public_height': stats['tip_height'], 'public_spec': public_spec['chain_id'],
                          'candidate_spec': candidate_spec['chain_id'],
                          'swagger': True}))
        break
    except Exception:
        if attempt == 11:
            raise
        time.sleep(1)
PY
rpc_payload=$(sudo -n docker exec "$review" curl -fsS -H 'Content-Type: application/json' \
  --data-binary '{"jsonrpc":"2.0","method":"eth_chainId","params":[],"id":14}' \
  http://127.0.0.1/rpc)
printf '%s' "$rpc_payload" | python3 -c "import json,sys; assert json.load(sys.stdin)['result'] == '0xaa36a7'"

switched=false
rollback() {
  status=$?
  if [ "$switched" = true ]; then
    if sudo -n docker inspect "$backup" >/dev/null 2>&1; then
      if sudo -n docker inspect creg-unified-explorer >/dev/null 2>&1; then
        test "$(sudo -n docker inspect creg-unified-explorer --format '{{.Config.Image}}')" = "$image" || exit 1
        sudo -n docker rm -f creg-unified-explorer
      fi
      sudo -n docker rename "$backup" creg-unified-explorer
      sudo -n docker start creg-unified-explorer
    elif sudo -n docker inspect creg-unified-explorer >/dev/null 2>&1; then
      test "$(sudo -n docker inspect creg-unified-explorer --format '{{.Image}}')" = "$old_image" || exit 1
      sudo -n docker start creg-unified-explorer
    else
      echo 'Rollback container is missing; refusing to continue.' >&2
      exit 1
    fi
  fi
  if [ -f "$compose_backup" ]; then cp -p "$compose_backup" "$compose"; fi
  if sudo -n docker inspect "$review" >/dev/null 2>&1; then sudo -n docker rm -f "$review"; fi
  exit "$status"
}
trap rollback ERR

cp -p "$compose" "$compose_backup"
switched=true
sudo -n docker stop creg-unified-explorer
sudo -n docker rename creg-unified-explorer "$backup"
sudo -n docker run -d --name creg-unified-explorer --restart unless-stopped \
  --network "$network" --network-alias web-explorer "$image"
sudo -n docker exec creg-unified-explorer nginx -t
sudo -n docker exec creg-unified-explorer curl -fsS http://127.0.0.1/health
python3 - <<'PY'
from pathlib import Path

path = Path('/home/samue/creg-hosting/chain-registry-blockchain-CREG-/chain-registry/testnet/docker-compose.unified.yml')
source = path.read_text()
old = 'image: chain-registry-web-explorer:pipelines-r13-20260926'
new = 'image: chain-registry-web-explorer:pipelines-r14-20260926'
assert source.count(old) == 1, 'Unexpected explorer image configuration'
source = source.replace(old, new)
old_comment = ('      # NOTE: VITE_* bake in at image build time; these runtime values are inert.\n'
               '      # The UI calls same-origin relative /v1/*, which nginx proxies to the node.\n')
new_comment = ('      # Chain contracts come from the signed spec matching the live node identity.\n'
               '      # Same-origin /v1/* and /rpc routes are served by nginx.\n')
assert source.count(old_comment) == 1, 'Unexpected explorer environment documentation'
source = source.replace(old_comment, new_comment)
for stale in (
    '      VITE_SEPOLIA_CREG_TOKEN: "${CREG_TOKEN_ADDR:-0x167Ab975dc95b79727Ed76BD8AAb70463F6499E2}"\n',
    '      VITE_SEPOLIA_STAKING_ADDR: "${CREG_STAKING_ADDR:-0x30604Aa6f763fbde6aDCffbd8960af3031037331}"\n',
    '      VITE_SEPOLIA_REGISTRY_ADDR: "${CREG_REGISTRY_ADDR:-0xD0F465ca185a99E268be0C4FF5A135Bdf89FAF3B}"\n',
):
    assert source.count(stale) == 1, 'Unexpected stale explorer contract environment setting'
    source = source.replace(stale, '')
path.write_text(source)
PY

sudo -n docker rm -f "$review"
trap - ERR
sudo -n docker inspect creg-unified-explorer --format '{{.Image}} {{.State.Status}}'
echo 'Web release applied. Public node, chain spec, and chain routing remain unchanged.'
