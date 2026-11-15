#!/usr/bin/env bash
# Roll observer-3 back to the r33 container. Detection writes nothing the r33
# binary reads, so no data step is needed; pass the backup directory printed by
# o3-swap-r34.sh only to also restore the pre-swap data copy.
# Usage: o3-rollback-r34.sh [BACKUP_DIR]
set -euo pipefail
backup=${1:-}
c=creg-phase-c-r7-o3; base=/var/lib/creg/phase-c-r7; data=$base/data
preserved=$c-r33-before-r34; ts=$(date -u +%Y%m%dT%H%M%SZ)
sudo docker container inspect "$preserved" >/dev/null
if [[ -n "$backup" ]]; then
  [[ "$backup" == "$base"/data-before-r34-* ]]
  sudo test -d "$backup/chain.rocksdb"
fi
sudo docker stop --time 20 "$c" >/dev/null 2>&1 || true
sudo docker rename "$c" "$c-r34-failed-$ts"
if [[ -n "$backup" ]]; then
  sudo mv "$data" "$base/data-r34-failed-$ts"
  sudo cp -a "$backup" "$data"
fi
sudo docker rename "$preserved" "$c"
sudo docker start "$c" >/dev/null
echo "rolled back: $c on $(sudo docker inspect "$c" --format '{{.Image}}'); r34 container kept as $c-r34-failed-$ts"
