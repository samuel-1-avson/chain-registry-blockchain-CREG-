#!/usr/bin/env bash
# Undo replace-o1-image.sh: stop the current o1 and bring back the container that
# ran before STAMP. Both share the data directory, so the current one stops first.
#
# Usage (on creg-testnet-vm): bash rollback-o1-image.sh STAMP
set -euo pipefail

stamp="${1:?the stamp printed by replace-o1-image.sh}"
c=creg-phase-c-r7-o1
previous="$c-before-$stamp"

DOCKER=(docker)
docker info >/dev/null 2>&1 || DOCKER=(sudo docker)
"${DOCKER[@]}" inspect "$previous" >/dev/null 2>&1 || { echo "[rollback-o1] no $previous container" >&2; exit 1; }

"${DOCKER[@]}" stop --time 30 "$c" >/dev/null 2>&1 || true
"${DOCKER[@]}" rename "$c" "$c-rolled-back-$stamp"
"${DOCKER[@]}" update --restart no "$c-rolled-back-$stamp" >/dev/null
"${DOCKER[@]}" rename "$previous" "$c"
"${DOCKER[@]}" update --restart unless-stopped "$c" >/dev/null
"${DOCKER[@]}" start "$c" >/dev/null
for _ in $(seq 1 60); do
  if curl -fsS -m 5 -o /dev/null http://127.0.0.1:18080/v1/public/chain/stats; then
    echo "[rollback-o1] previous $c restored and answering (the replaced one is kept, stopped, as $c-rolled-back-$stamp)"
    echo "[rollback-o1] ROLLBACK-OK"
    exit 0
  fi
  sleep 3
done
echo "[rollback-o1] previous $c started but its API is not answering yet; check: docker logs $c" >&2
exit 1
