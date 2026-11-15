#!/usr/bin/env bash
# Read-only. Usage: peer-seen.sh CONTAINER PORT SINCE PEER_ID...
set -u
c=$1; port=$2; since=$3; shift 3
logs=$(sudo docker logs --since "$since" "$c" 2>&1 | sed -E 's/\x1b\[[0-9;]*m//g')
for peer in "$@"; do
  echo "$c: connections from ${peer:0:16}... since $since: $(grep -c "Connection established with $peer" <<<"$logs")"
done
echo "$c: peer_count now: $(curl -fsS --max-time 8 http://127.0.0.1:$port/v1/public/chain/stats | python3 -c 'import json,sys; print(json.load(sys.stdin)["peer_count"])')"
