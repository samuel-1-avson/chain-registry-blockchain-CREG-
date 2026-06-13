#!/usr/bin/env bash
# Remote checks for verify-monitoring.ps1 (run on edge VM).
set -euo pipefail

curl -fsS http://127.0.0.1:9090/-/healthy >/dev/null
echo "PROMETHEUS_OK"

targets_json="$(curl -fsS 'http://127.0.0.1:9090/api/v1/targets')"
if echo "$targets_json" | grep -q '"health":"up"'; then
  echo "TARGETS_UP"
else
  echo "NO_UP_TARGETS"
  exit 3
fi

metrics="$(curl -fsS 'http://127.0.0.1:9090/api/v1/query?query=creg_chain_tip_height')"
if echo "$metrics" | grep -q '"status":"success"'; then
  echo "METRICS_OK"
else
  echo "METRICS_QUERY_FAILED"
  exit 4
fi

sandbox="$(curl -fsS 'http://127.0.0.1:9090/api/v1/query?query=creg_sandbox_dev_bypass')"
if echo "$sandbox" | grep -q '"status":"success"'; then
  echo "SANDBOX_METRICS_OK"
else
  echo "SANDBOX_METRICS_MISSING"
  exit 5
fi

alerts="$(curl -fsS 'http://127.0.0.1:9090/api/v1/rules')"
if echo "$alerts" | grep -q 'CregSandboxDevBypass'; then
  echo "ALERT_RULES_OK"
else
  echo "ALERT_RULES_MISSING"
  exit 6
fi
