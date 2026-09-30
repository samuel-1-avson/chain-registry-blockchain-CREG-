#!/usr/bin/env bash
# Guarded one-validator rollout on the real R7 V4 VM. The other three
# validators stay online; existing chain data and signing journal are mounted.
set -euo pipefail

base=/var/lib/creg/phase-c-r7
container=creg-phase-c-r7-v4
preserved=${container}-r22-before-r25
image=creg-node:phase-c-r25-follower-capture
image_id=sha256:704cd7dfb89a048c146a3de502416c31a6f77c0e3f493a98f4fe3098c732046c
binary_sha=24925bf0df62ed9fafe4565f9969ba83c0beefbbd6725abbd8814f99ea04cf07
scanner=$base/scanner-r11-osv-kleur
evidence=$base/evidence
verifier=/tmp/verify-r25-v4-validator.py

exec 9>/tmp/creg-r25-v4-rollout.lock
flock -n 9 || { echo 'V4 rollout already in progress' >&2; exit 1; }
sudo test -f "$verifier"
sudo test -d "$base/data/pbft-safety.rocksdb"
sudo test -d "$scanner"
[[ $(sudo docker image inspect -f '{{.Id}}' "$image") == "$image_id" ]]
[[ $(sudo docker run --rm "$image" --version) == 'creg-node 0.1.0' ]]
[[ $(sudo docker run --rm --entrypoint sha256sum "$image" /app/creg-node | cut -d' ' -f1) == "$binary_sha" ]]
! sudo docker container inspect "$preserved" >/dev/null 2>&1
sudo test ! -e "$evidence/r25-v4-validator-preflight.json"
sudo test ! -e "$evidence/r25-v4-validator-rollout.json"

sudo python3 "$verifier" pre >/tmp/creg-r25-v4-preflight.json
sudo install -m 0644 /tmp/creg-r25-v4-preflight.json "$evidence/r25-v4-validator-preflight.json"
cat /tmp/creg-r25-v4-preflight.json

sudo docker stop --time 30 "$container" >/dev/null
sudo docker rename "$container" "$preserved"
docker_group=$(stat -c '%g' /var/run/docker.sock)
sudo docker run -d --name "$container" --restart unless-stopped \
  --env-file "$base/node.env" -p 8080:8080 -p 9000:9000 \
  --mount "type=bind,source=$base/data,target=/data" \
  --mount "type=bind,source=$scanner,target=/scanner,readonly" \
  --privileged --group-add "$docker_group" \
  --mount type=bind,source=/var/run/docker.sock,target=/var/run/docker.sock \
  "$image" >/dev/null

deadline=$((SECONDS + 360))
until sudo python3 "$verifier" post >/tmp/creg-r25-v4-post-check.json 2>/tmp/creg-r25-v4-post-error.log; do
  if (( SECONDS >= deadline )); then
    sudo install -m 0644 /tmp/creg-r25-v4-post-check.json "$evidence/r25-v4-validator-failed-post-check.json"
    echo 'V4 post-check did not pass; newer image and prior container retained for review; do not start the older validator against newer signing state' >&2
    cat /tmp/creg-r25-v4-post-error.log >&2
    exit 1
  fi
  sleep 3
done
[[ $(sudo docker inspect -f '{{.State.Status}}' "$preserved") == exited ]]
sudo install -m 0644 /tmp/creg-r25-v4-post-check.json "$evidence/r25-v4-validator-rollout.json"
cat /tmp/creg-r25-v4-post-check.json
