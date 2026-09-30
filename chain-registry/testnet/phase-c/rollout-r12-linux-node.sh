#!/usr/bin/env bash
# Replace one real r7 candidate container while preserving its data, signer
# journal, scanner mount, and P2P identity.
set -euo pipefail

slot=${1:?slot required}
role=${2:?role required}
api_port=${3:?host API port required}
p2p_port=${4:?host P2P port required}
allow_api_down=${5:-false}

case "$slot:$role" in
    v[1-4]:validator|o[1-4]:observer) ;;
    *) echo 'Invalid candidate slot or role' >&2; exit 2 ;;
esac
case "$api_port:$p2p_port" in
    8080:9000|18080:9001) ;;
    *) echo 'Unexpected candidate port mapping' >&2; exit 2 ;;
esac
if [[ "$allow_api_down" != true && "$allow_api_down" != false ]]; then
    echo 'allow_api_down must be true or false' >&2
    exit 2
fi

image=creg-node:phase-c-secure-r12-bounded-l1-retry
expected_image=sha256:e4790446f65de6e1404200bfe8d3a2e444cb80a997a16cdef969da2bfd62e2dd
expected_index=sha256:f0b693dc567fc3430f22db881581c55a253e75c8f84a5a0f2d6a82d4460d0736
expected_tip=0797e80bf476d30b47fe2331e3274e55dc0c37d35ffbe6420001945bffbf47e0
container=creg-phase-c-r7-${slot}
base=/var/lib/creg/phase-c-r7
data_dir=$base/data
scanner_dir=$base/scanner
env_path=$base/node.env
identity_path=$data_dir/p2p/identity.key

if ! sudo test -f "$env_path" || ! sudo test -f "$identity_path" ||
   ! sudo test -d "$data_dir" || ! sudo test -d "$scanner_dir"; then
    echo 'Existing candidate environment, identity, data, or scanner bundle is missing' >&2
    exit 1
fi
if [[ "$role" == validator ]] && ! sudo test -d "$data_dir/pbft-safety.rocksdb"; then
    echo 'Validator signing journal is missing; refusing replacement' >&2
    exit 1
fi

case "$slot" in
    v1) node_id=validator-1 ;;
    v2) node_id=validator-2 ;;
    v3) node_id=validator-3 ;;
    v4) node_id=validator-4 ;;
    o1) node_id=observer-1 ;;
    o2) node_id=observer-2 ;;
    o3) node_id=observer-3 ;;
    o4) node_id=observer-4 ;;
esac
is_validator=false
if [[ "$role" == validator ]]; then is_validator=true; fi
sudo grep -qx "CREG_NODE_ID=$node_id" "$env_path"
sudo grep -qx "CREG_NODE_ROLE=$role" "$env_path"
sudo grep -qx "CREG_IS_VALIDATOR=$is_validator" "$env_path"
sudo grep -qx 'CREG_PBFT_ALLOW_SMALL_CLUSTER_QUORUM=false' "$env_path"
printf '%s  %s\n' \
    5e3a7beac3c5b7a5d766c30ff024509bc1df23b3f7c5d74349cc54ed6bc960ef \
    "$scanner_dir/known_malicious_hashes.json" |
    sudo sha256sum -c - >/dev/null
printf '%s  %s\n' \
    39d74eb091cbe986073f3a711c2f25f3be6975dbb38e2083e5fe5e445e7d4ade \
    "$scanner_dir/osv_snapshot.json" |
    sudo sha256sum -c - >/dev/null
printf '%s  %s\n' \
    217b121bc5943e61d483de8f15d89be9bb91b7b1d3e18d5f18b8396f59f2683e \
    "$scanner_dir/rules/supply_chain.yar" |
    sudo sha256sum -c - >/dev/null

image_id=$(sudo docker image inspect "$image" --format '{{.Id}}')
if [[ "$image_id" != "$expected_image" && "$image_id" != "$expected_index" ]]; then
    echo 'Loaded r12 image does not match the pinned build config digest' >&2
    exit 1
fi
if ! sudo docker container inspect "$container" >/dev/null 2>&1; then
    echo 'Existing candidate container is missing; refusing fresh launch' >&2
    exit 1
fi
state=$(sudo docker inspect "$container" --format '{{.State.Status}}')
if [[ "$state" != running ]]; then
    echo "Existing candidate process is not running; state=$state" >&2
    exit 1
fi
prior_image=$(sudo docker inspect "$container" --format '{{.Config.Image}}')
case "$prior_image" in
    *r11*) prior_version=r11 ;;
    *r10*) prior_version=r10 ;;
    *r9*) prior_version=r9 ;;
    *) echo "Unexpected prior image $prior_image" >&2; exit 1 ;;
esac
preserved=${container}-${prior_version}-preserved
if sudo docker container inspect "$preserved" >/dev/null 2>&1; then
    echo "Preserved container already exists: $preserved" >&2
    exit 1
fi
mounts=$(sudo docker inspect "$container" --format '{{range .Mounts}}{{println .Source .Destination}}{{end}}')
grep -Fxq "$data_dir /data" <<<"$mounts"
grep -Fxq "$scanner_dir /scanner" <<<"$mounts"

api_url=http://127.0.0.1:${api_port}
if stats=$(curl -fsS --max-time 5 "$api_url/v1/public/chain/stats"); then
    python3 - "$stats" "$expected_tip" <<'PY'
import json, sys
stats = json.loads(sys.argv[1])
if (stats.get('chain_id') != 'creg-phase-c-testnet-2026-09-26-r7'
        or stats.get('tip_height') != 4
        or stats.get('tip_hash') != sys.argv[2]
        or stats.get('spec_signature_verified') is not True):
    raise SystemExit('Pre-rollout chain identity or h4 tip mismatch')
PY
elif [[ "$allow_api_down" != true ]]; then
    echo 'Candidate API is unavailable; inspect before replacing this node' >&2
    exit 1
fi

identity_before=$(sudo sha256sum "$identity_path" | awk '{print $1}')
sudo docker stop --time 20 "$container" >/dev/null
if [[ $(sudo docker inspect "$container" --format '{{.State.Status}}') != exited ]]; then
    echo 'Candidate did not stop cleanly; refusing image replacement' >&2
    exit 1
fi
sudo docker rename "$container" "$preserved"

args=(run -d --name "$container" --restart unless-stopped
    --env-file "$env_path"
    -p "$api_port:8080" -p "$p2p_port:9000"
    --mount "type=bind,source=$data_dir,target=/data"
    --mount "type=bind,source=$scanner_dir,target=/scanner,readonly")
if [[ "$role" == validator ]]; then
    docker_group=$(stat -c '%g' /var/run/docker.sock)
    args+=(--privileged --group-add "$docker_group" -v /var/run/docker.sock:/var/run/docker.sock)
fi
if ! sudo docker "${args[@]}" "$image" >/dev/null; then
    echo 'r12 container creation failed; prior stopped container is preserved for inspection' >&2
    exit 1
fi
actual_image=$(sudo docker inspect "$container" --format '{{.Image}}')
if [[ "$actual_image" != "$expected_image" && "$actual_image" != "$expected_index" ]]; then
    echo 'Running node image does not match the pinned r12 build' >&2
    exit 1
fi
identity_after=$(sudo sha256sum "$identity_path" | awk '{print $1}')
if [[ "$identity_before" != "$identity_after" ]]; then
    echo 'P2P identity changed during image replacement' >&2
    exit 1
fi
if [[ "$role" == validator ]] && ! sudo test -d "$data_dir/pbft-safety.rocksdb"; then
    echo 'PBFT signing journal is missing after replacement' >&2
    exit 1
fi

deadline=$((SECONDS + 360))
until curl -fsS --max-time 3 "$api_url/v1/public/health" >/dev/null; do
    if (( SECONDS >= deadline )); then
        echo 'r12 process did not bind its API within six minutes; leave both containers preserved for inspection' >&2
        exit 1
    fi
    sleep 2
done
echo "r12 $slot process is serving; image=$actual_image prior=$preserved p2p_identity=preserved signing_journal=$([[ "$role" == validator ]] && echo preserved || echo not_applicable)"
