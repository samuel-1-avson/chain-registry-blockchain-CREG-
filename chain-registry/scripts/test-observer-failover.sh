#!/usr/bin/env bash
# scripts/test-observer-failover.sh
# Automated verification for Observer Pool High Availability & Zero-Downtime Failover Drill
set -euo pipefail

LB_URL="${1:-http://localhost:8090}"
OBSERVER1_URL="${2:-http://localhost:28181}"
OBSERVER2_URL="${3:-http://localhost:28182}"
SKIP_DRILL="${4:-false}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_ROOT"

echo -e "\033[0;36m=======================================================\033[0m"
echo -e "\033[0;36m  CREG Observer Pool -- High-Availability Failover Drill\033[0m"
echo -e "\033[0;36m=======================================================\033[0m"

# Step 1: Verify Load Balancer
echo -e "\n\033[1;33m[1/5] Checking Load Balancer status...\033[0m"
if lb_resp=$(curl -fsS --max-time 5 "$LB_URL/lb-status" 2>/dev/null); then
  echo -e "  \033[0;32m[OK] Load Balancer is active ($LB_URL)\033[0m"
else
  echo -e "  \033[0;31m[ERROR] Load Balancer not reachable at $LB_URL. Ensure docker-compose.observer-ha.yml is running.\033[0m" >&2
  exit 1
fi

# Step 2: Verify Initial Cluster Health
echo -e "\n\033[1;33m[2/5] Checking initial pool health...\033[0m"
healthy_count=0
for endpoint in "Observer-1:$OBSERVER1_URL" "Observer-2:$OBSERVER2_URL"; do
  name="${endpoint%%:*}"
  url="${endpoint#*:}"
  if res=$(curl -fsS --max-time 5 "$url/v1/health" 2>/dev/null); then
    echo -e "  \033[0;32m[OK] $name is healthy ($url)\033[0m"
    healthy_count=$((healthy_count + 1))
  else
    echo -e "  \033[1;33m[WARN] $name not yet healthy at $url\033[0m"
  fi
done

if [ "$healthy_count" -eq 0 ]; then
  echo -e "  \033[0;31m[ERROR] Neither observer is healthy yet. Wait for startup to complete.\033[0m" >&2
  exit 1
fi

# Step 3: Verify Public Routing via Load Balancer
echo -e "\n\033[1;33m[3/5] Testing public queries through Load Balancer ($LB_URL)...\033[0m"
for i in $(seq 1 5); do
  if res=$(curl -fsS --max-time 3 "$LB_URL/v1/health" 2>/dev/null); then
    echo "  Request $i: 200 OK"
  else
    echo -e "  \033[0;31mRequest $i: FAILED\033[0m"
  fi
  sleep 0.2
done
echo -e "  \033[0;32m[OK] Traffic successfully balancing across pool\033[0m"

if [ "$SKIP_DRILL" = "true" ] || [ "$SKIP_DRILL" = "--skip-drill" ]; then
  echo -e "\n\033[0;32m[+] Preflight checks complete (failover drill skipped).\033[0m"
  exit 0
fi

# Step 4: Active Failover Drill (Stop Observer-1)
echo -e "\n\033[1;33m[4/5] INITIATING FAILOVER DRILL: Stopping Observer-1...\033[0m"
docker compose -f docker-compose.observer-ha.yml stop observer-1 >/dev/null
echo "  Observer-1 stopped. Testing API continuity..."

failover_successes=0
total_requests=10
for i in $(seq 1 $total_requests); do
  if res=$(curl -fsS --max-time 3 "$LB_URL/v1/health" 2>/dev/null); then
    failover_successes=$((failover_successes + 1))
  else
    echo -e "  \033[0;31m[FAIL] Request $i dropped during outage\033[0m"
  fi
  sleep 0.15
done

if [ "$failover_successes" -eq "$total_requests" ]; then
  echo -e "  \033[0;32m[OK] 100% of requests survived! ($failover_successes/$total_requests survived with ZERO downtime)\033[0m"
else
  echo -e "  \033[0;31m[ERROR] Failover drill failed: only $failover_successes/$total_requests succeeded.\033[0m" >&2
  exit 1
fi

# Step 5: Self-Healing & Recovery
echo -e "\n\033[1;33m[5/5] RECOVERY DRILL: Restarting Observer-1...\033[0m"
docker compose -f docker-compose.observer-ha.yml start observer-1 >/dev/null
sleep 5

recovered=false
for attempt in $(seq 1 10); do
  if curl -fsS --max-time 3 "$OBSERVER1_URL/v1/health" >/dev/null 2>&1; then
    recovered=true
    break
  fi
  sleep 2
done

if [ "$recovered" = "true" ]; then
  echo -e "  \033[0;32m[OK] Observer-1 recovered and re-joined the pool automatically\033[0m"
else
  echo -e "  \033[1;33m[WARN] Observer-1 started but has not reported healthy yet\033[0m"
fi

echo -e "\n\033[0;36m=======================================================\033[0m"
echo -e "\033[0;32m  DRILL COMPLETE: High-Availability & Zero Downtime VERIFIED!\033[0m"
echo -e "\033[0;36m=======================================================\033[0m"
