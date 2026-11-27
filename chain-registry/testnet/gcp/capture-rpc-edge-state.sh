#!/usr/bin/env bash
# Read-only capture of the live RPC edge on the VM
# (docs/rpc-remediation-plan.md, Phase 0 tasks 0.1, 0.3 and 0.5).
#
# Changes nothing: no restarts, no reloads, no writes outside OUT_DIR.
#
# Usage, on the VM that runs Caddy:
#   bash capture-rpc-edge-state.sh [OUT_DIR]
#
# OUT_DIR keeps full copies of the Caddy files and the active Caddy JSON config.
# Those can contain secrets (for example a provider key in a rewrite) and the ACME
# email, so the directory is created mode 700 and should stay on the VM. Share only
# summary.txt: it reduces every URL to scheme://host[:port].
set -euo pipefail
umask 077

out="${1:-$HOME/creg-rpc-edge-state-$(date -u +%Y%m%dT%H%M%SZ)}"
mkdir -p "$out"
repo="${CREG_REPO:-$HOME/creg-hosting/chain-registry-blockchain-CREG-}"

DOCKER=(docker)
if ! docker info >/dev/null 2>&1; then
  DOCKER=(sudo docker)
fi

# Keep scheme://host[:port]; drop userinfo, path and query, where provider keys live.
host_only() { sed -E 's#^([A-Za-z][A-Za-z0-9+.-]*://)?([^@/]*@)?([^/?#]*).*#\1\3#'; }
hosts_csv() { tr ',' '\n' | sed '/^[[:space:]]*$/d' | host_only | paste -sd, -; }
names() { "${DOCKER[@]}" ps --format '{{.Names}}' | grep -E "$1" || true; }
penv() { "${DOCKER[@]}" exec "$1" printenv "$2" 2>/dev/null || true; }
ips() { "${DOCKER[@]}" inspect -f '{{range $n, $v := .NetworkSettings.Networks}}{{$n}}={{$v.IPAddress}} {{end}}' "$1"; }
mounts() { "${DOCKER[@]}" inspect -f '{{range .Mounts}}{{.Source}} -> {{.Destination}}; {{end}}' "$1"; }
same_text() { diff -q <(tr -d '\r' <"$1") <(tr -d '\r' <"$2") >/dev/null 2>&1; }

caddy_containers=$(names 'caddy')
explorer_containers=$(names 'explorer')
node_containers=$(names 'creg-phase-c-r7-o|creg-unified-node|observer')
untrusted_nodes=()

{
  echo "captured_at $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "vm $(hostname)"
  if git -C "$repo" rev-parse HEAD >/dev/null 2>&1; then
    echo "vm_repo_head $(git -C "$repo" rev-parse --short HEAD)"
    echo "vm_repo_caddy_uncommitted_files $(git -C "$repo" status --porcelain -- chain-registry/testnet/caddy | wc -l)"
  else
    echo "vm_repo_head <no git checkout at $repo>"
  fi

  echo
  echo "== containers"
  "${DOCKER[@]}" ps --format '{{.Names}}\t{{.Image}}\t{{.Status}}'

  for c in $caddy_containers; do
    echo
    echo "== caddy $c"
    echo "networks $(ips "$c")"
    echo "mounts $(mounts "$c")"
    version=$("${DOCKER[@]}" exec "$c" caddy version 2>/dev/null || true)
    echo "version ${version:-unknown}"
    for f in Caddyfile sepolia-public-rpc.caddy faucet-edge.caddy; do
      if "${DOCKER[@]}" exec "$c" test -f "/etc/caddy/$f"; then
        "${DOCKER[@]}" exec "$c" cat "/etc/caddy/$f" >"$out/$c.$f"
        echo "file $f sha256 $(tr -d '\r' <"$out/$c.$f" | sha256sum | cut -c1-16)"
      fi
    done
    repo_unified="$repo/chain-registry/testnet/caddy/Caddyfile.unified"
    repo_sepolia="$repo/chain-registry/testnet/caddy/sepolia-public-rpc.caddy"
    if [[ -f "$out/$c.Caddyfile" && -f "$repo_unified" ]]; then
      if same_text "$out/$c.Caddyfile" "$repo_unified"; then
        echo "mounted Caddyfile matches VM repo Caddyfile.unified"
      else
        echo "mounted Caddyfile DIFFERS from VM repo Caddyfile.unified"
      fi
    fi
    if [[ -f "$out/$c.sepolia-public-rpc.caddy" && -f "$repo_sepolia" ]]; then
      if same_text "$out/$c.sepolia-public-rpc.caddy" "$repo_sepolia"; then
        echo "mounted sepolia-public-rpc.caddy matches VM repo copy"
      else
        echo "mounted sepolia-public-rpc.caddy DIFFERS from VM repo copy"
      fi
    fi
    # The admin API serves the config Caddy is actually running, which can differ
    # from the files if they changed without a reload.
    if "${DOCKER[@]}" exec "$c" wget -qO- http://127.0.0.1:2019/config/ >"$out/$c.active-config.json" 2>/dev/null; then
      dials=$(grep -o '"dial":"[^"]*"' "$out/$c.active-config.json" | sed 's/"dial"://; s/"//g' | sort -u | paste -sd' ' - || true)
      echo "active_upstreams ${dials:-none}"
      if grep -q 'allowlisted' "$out/$c.active-config.json"; then
        echo "active_config_mentions_allowlist yes"
      else
        echo "active_config_mentions_allowlist no (pre-ECA-38 descriptors are loaded)"
      fi
      if grep -qi 'x-creg-edge-hop' "$out/$c.active-config.json"; then
        echo "active_config_routes_sepolia_via_explorer yes"
      else
        echo "active_config_routes_sepolia_via_explorer no"
      fi
    else
      rm -f "$out/$c.active-config.json"
      echo "active_config <admin API on 127.0.0.1:2019 not reachable>"
    fi
  done

  for c in $explorer_containers; do
    echo
    echo "== explorer $c"
    echo "image $("${DOCKER[@]}" inspect -f '{{.Config.Image}}' "$c")"
    echo "networks $(ips "$c")"
    echo "mounts $(mounts "$c")"
    if "${DOCKER[@]}" exec "$c" test -f /etc/nginx/njs/rpc-allowlist.js; then
      echo "allowlist_module present"
    else
      echo "allowlist_module MISSING (image predates ADR-019 Increment 1)"
    fi
    proxies=$("${DOCKER[@]}" exec "$c" grep -c 'rpc.proxy' /etc/nginx/conf.d/default.conf 2>/dev/null) || true
    echo "rpc_proxy_lines_in_default_conf ${proxies:-0}"
    echo "sepolia_upstream $(penv "$c" CREG_SEPOLIA_RPC_UPSTREAM | host_only)"
    echo "sepolia_host_header $(penv "$c" CREG_SEPOLIA_RPC_HOST)"
    echo "node_upstream $(penv "$c" CREG_NODE_UPSTREAM)"
  done

  for c in $node_containers; do
    echo
    echo "== node $c"
    echo "image $("${DOCKER[@]}" inspect -f '{{.Config.Image}}' "$c")"
    echo "networks $(ips "$c")"
    trusted=$(penv "$c" CREG_TRUSTED_PROXIES)
    echo "CREG_TRUSTED_PROXIES ${trusted:-<unset>}"
    [[ -z "$trusted" ]] && untrusted_nodes+=("$c")
    for var in CREG_ETH_RPC CREG_ETH_RPC_URL; do
      value=$(penv "$c" "$var")
      if [[ -n "$value" ]]; then
        echo "$var $(printf '%s' "$value" | host_only)"
      else
        echo "$var <unset>"
      fi
    done
    fallbacks=$(penv "$c" CREG_ETH_RPC_FALLBACKS)
    if [[ -n "$fallbacks" ]]; then
      echo "CREG_ETH_RPC_FALLBACKS $(printf '%s' "$fallbacks" | hosts_csv)"
    else
      echo "CREG_ETH_RPC_FALLBACKS <unset>"
    fi
  done

  echo
  echo "== verdicts"
  if [[ -z "$caddy_containers" ]]; then echo "no Caddy container found on this VM"; fi
  for c in ${untrusted_nodes[@]+"${untrusted_nodes[@]}"}; do
    echo "R-04: $c has no CREG_TRUSTED_PROXIES, so it rate-limits on its TCP peer. Every client that arrives through Caddy or the explorer shares one bucket."
  done
  echo
  echo "Full copies are in $out. They may contain secrets: keep them on the VM. Share only summary.txt."
} 2>&1 | tee "$out/summary.txt"
