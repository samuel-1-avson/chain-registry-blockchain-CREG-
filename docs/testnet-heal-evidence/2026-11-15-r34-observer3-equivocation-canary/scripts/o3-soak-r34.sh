#!/usr/bin/env bash
# Read-only soak check of the r34 canary on observer-3. Changes nothing.
set -u
c=creg-phase-c-r7-o3; api=http://127.0.0.1:8080
echo "== container"
sudo docker inspect -f '{{.Image}} {{.State.Status}} restarts={{.RestartCount}} started={{.State.StartedAt}} health={{.State.Health.Status}}' "$c"
echo "== stats"
curl -fsS --max-time 8 "$api/v1/public/chain/stats" | python3 -c 'import json,sys; s=json.load(sys.stdin); print({k:s.get(k) for k in ("tip_height","tip_hash","finalized_height","peer_count","active_validators","package_count","spec_signature_verified")})'
echo "== equivocation endpoint"
curl -fsS --max-time 8 "$api/v1/public/evidence/equivocation"; echo
echo "== equivocation metrics"
curl -fsS --max-time 8 "$api/metrics" | grep -E '^creg_equivocation_'
echo "== logs since start"
# Strip ANSI colour codes first; the node logs with colours.
logs=$(sudo docker logs "$c" 2>&1 | sed -E 's/\x1b\[[0-9;]*m//g')
echo "lines=$(wc -l <<<"$logs")"
echo "== lines per level"
awk '{print $2}' <<<"$logs" | grep -E '^(TRACE|DEBUG|INFO|WARN|ERROR)$' | sort | uniq -c
echo "journal_warning=$(grep -c 'durable safety journal is not initialized' <<<"$logs")"
echo "equivocation_lines=$(grep -ci 'equivocation' <<<"$logs")"
grep -i 'equivocation' <<<"$logs" | tail -5
echo "== distinct WARN/ERROR messages (whole log, first 140 chars after the target)"
awk '$2=="WARN" || $2=="ERROR"' <<<"$logs" | cut -d' ' -f2- | cut -c1-170 | sed -E 's/[0-9a-f]{16,}/<hex>/g; s/[0-9]+/N/g' | sort | uniq -c | sort -rn | head -12
echo "== distinct INFO messages (whole log)"
awk '$2=="INFO"' <<<"$logs" | cut -d' ' -f3- | cut -c1-120 | sed -E 's/[0-9a-f]{16,}/<hex>/g; s/[0-9]+/N/g' | sort | uniq -c | sort -rn | head -12
echo "== r33 container for comparison: journal warning lines in its last 10 minutes of life"
sudo docker logs --since 2026-11-15T09:39:00Z --until 2026-11-15T09:49:30Z creg-phase-c-r7-o3-r33-before-r34 2>&1 | grep -c 'durable safety journal is not initialized'
echo "== evidence dir in data"
sudo ls -la /var/lib/creg/phase-c-r7/data/equivocation-evidence 2>&1 | head -5
echo "== done"
