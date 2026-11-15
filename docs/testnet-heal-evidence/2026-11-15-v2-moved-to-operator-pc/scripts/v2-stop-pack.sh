#!/usr/bin/env bash
# Stop validator-2 on the VM for good (restart=no) and pack its state.
# If anything fails after the stop, v2 is restarted here so the fleet keeps it.
set -euo pipefail
c=creg-phase-c-r7-v2; stray=creg-validator-validator-1; base=/var/lib/creg/phase-c-r7
api=http://127.0.0.1:18080; tarf=/tmp/v2-move.tar
img=sha256:2b472a089fe6ac651edfbae51b616456ed335f35992c0374f10c1ea80ce6caa3
exec 9>/tmp/creg-v2-move.lock; flock -n 9
[[ $(sudo docker inspect -f '{{.Image}} {{.State.Status}}' "$c") == "$img running" ]]
python3 - "$api" <<'PY'
import json, sys, urllib.request
api = sys.argv[1]
g = lambda p: json.loads(urllib.request.urlopen(api + p, timeout=8).read())
s = g('/v1/public/chain/stats'); r = g('/v1/consensus/pbft')
assert s['tip_height'] == 20 and s['finalized_height'] == 20, s
assert s['tip_hash'] == '730b01c1a67a4796a16f599169e4ebf77e01d2e8ce92d1e4fb3715a7c4354285', s
assert not r.get('rounds'), 'PBFT round in progress'
print('pre ok: tip', s['tip_height'], 'peers', s['peer_count'])
PY
if [[ -e "$tarf" ]]; then echo "$tarf already exists"; exit 1; fi
stopped=false
restore() {
  st=$?
  if (( st != 0 )) && [[ $stopped == true ]]; then
    echo "failure after stop; restarting v2 on the VM"
    sudo rm -f "$tarf"
    sudo docker update --restart unless-stopped "$c" >/dev/null || true
    sudo docker start "$c" >/dev/null || true
  fi
  exit "$st"
}
trap restore EXIT
sudo docker update --restart=no "$c" "$stray" >/dev/null
stopped=true
sudo docker stop --time 30 "$c" >/dev/null
sudo docker stop --time 10 "$stray" >/dev/null || true
[[ $(sudo docker inspect -f '{{.State.Running}}' "$c") == false ]]
[[ $(sudo docker inspect -f '{{.State.Running}}' "$stray") == false ]]
echo "stopped_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
sudo tar -C "$base" -cf "$tarf" data node.env scanner-r14-ansi-regex-live
sudo chown "$(id -u):$(id -g)" "$tarf"
chmod 600 "$tarf"
echo "tar_sha256=$(sha256sum "$tarf" | cut -d' ' -f1)"
echo "tar_bytes=$(stat -c %s "$tarf")"
echo "identity_fp=$(sudo sha256sum "$base/data/p2p/identity.key" | cut -c1-16)"
echo "data_files=$(sudo find "$base/data" -type f | wc -l)"
echo "restart_policy=$(sudo docker inspect -f '{{.HostConfig.RestartPolicy.Name}}' "$c")"
trap - EXIT
echo "PACKED: v2 stopped on the VM with restart=no"
