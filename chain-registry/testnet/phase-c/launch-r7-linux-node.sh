#!/usr/bin/env bash
# Start one isolated r7 node with the pinned scanner bundle and fresh data.
set -euo pipefail

slot=${1:?slot required}
role=${2:?role required}
host_api_port=${3:?host API port required}
host_p2p_port=${4:?host P2P port required}
image=${5:?immutable image reference required}

expected_index_id=sha256:cf9fda37887e0d8c6125280c07bdb36737e495e75d070b80abcc722187180d33
expected_config_id=sha256:e25b3470e0372b22c5bb98a91cb40a9449051d51cddd366ff8f957cd983f447b
expected_genesis=0x1852b9a86dd3b53e7b2de4a774723a4fe48e862f9c61d14e5917b42b21055234
expected_profile=648dfd39a02da6dd8c42f2342146208efeae1f435e18d2fcd71317828062fb0f
expected_epoch=osv-2026-09-26-phase-c-r8
container=creg-phase-c-r7-${slot}
base=/var/lib/creg/phase-c-r7
scanner_dir=$base/scanner
data_dir=$base/data
env_path=$base/node.env
stage=$HOME/.creg-phase-c-r7-${slot}

case "$slot:$role" in
    v[1-4]:validator|o[1-4]:observer) ;;
    *) echo 'Invalid r7 slot or role' >&2; exit 2 ;;
esac
case "$host_api_port:$host_p2p_port" in
    8080:9000|18080:9001) ;;
    *) echo 'Unexpected host port mapping' >&2; exit 2 ;;
esac

image_id=$(sudo docker image inspect "$image" --format '{{.Id}}')
if [ "$image_id" != "$expected_index_id" ] && [ "$image_id" != "$expected_config_id" ]; then
    echo 'r9 image config digest mismatch' >&2
    exit 1
fi
if ! test -f "$stage/node.env" || ! test -f "$stage/identity.key"; then
    echo 'Private r7 node environment and P2P identity must be staged in the protected home directory' >&2
    exit 1
fi
if [ "$(stat -c '%a' "$stage")" != 700 ] ||
   [ "$(stat -c '%a' "$stage/node.env")" != 600 ] ||
   [ "$(stat -c '%a' "$stage/identity.key")" != 600 ]; then
    echo 'Private staging permissions are too broad' >&2
    exit 1
fi

expected_validator=false
if [ "$role" = validator ]; then expected_validator=true; fi
if [ "$role" = validator ]; then
    expected_node_id=validator-${slot#v}
else
    expected_node_id=observer-${slot#o}
fi
grep -qx "CREG_NODE_ID=$expected_node_id" "$stage/node.env"
grep -qx "CREG_NODE_ROLE=$role" "$stage/node.env"
grep -qx "CREG_IS_VALIDATOR=$expected_validator" "$stage/node.env"
grep -qx "CREG_GENESIS_HASH=$expected_genesis" "$stage/node.env"
grep -qx 'CREG_CHAIN_ID=creg-phase-c-testnet-2026-09-26-r7' "$stage/node.env"
grep -qx 'CREG_CHAIN_SPEC_URL=https://spec.testnet.cregnet.dev/phase-c/r7/chain-spec.json' "$stage/node.env"
grep -qx 'CREG_SPEC_SIGNATURE_URL=https://spec.testnet.cregnet.dev/phase-c/r7/chain-spec.json.sig' "$stage/node.env"
grep -qx "CREG_SCANNER_PROFILE_DIGEST=$expected_profile" "$stage/node.env"
grep -qx "CREG_OSV_SNAPSHOT_EPOCH=$expected_epoch" "$stage/node.env"
grep -qx 'CREG_OSV_CONSENSUS=true' "$stage/node.env"
grep -qx 'CREG_OSV_BLOCK_CRITICAL=true' "$stage/node.env"
grep -qx 'CREG_OSV_LIVE_FALLBACK=false' "$stage/node.env"
grep -qx 'CREG_YARA_RULES_DIR=/scanner/rules' "$stage/node.env"
grep -qx 'CREG_THREAT_DB_PATH=/scanner/known_malicious_hashes.json' "$stage/node.env"
grep -qx 'CREG_OSV_SNAPSHOT_PATH=/scanner/osv_snapshot.json' "$stage/node.env"
grep -qx 'CREG_PBFT_ALLOW_SMALL_CLUSTER_QUORUM=false' "$stage/node.env"
grep -qx 'CREG_DEV_SANDBOX=false' "$stage/node.env"
if [ "$role" = validator ]; then
    grep -q '^CREG_VALIDATOR_KEY=' "$stage/node.env"
elif grep -q '^CREG_VALIDATOR_KEY=' "$stage/node.env"; then
    echo 'Observer environment must not contain a validator signing key' >&2
    exit 1
fi

if ! sudo test -f "$scanner_dir/known_malicious_hashes.json" ||
   ! sudo test -f "$scanner_dir/osv_snapshot.json" ||
   ! sudo test -f "$scanner_dir/rules/supply_chain.yar"; then
    echo 'The pinned scanner bundle is incomplete' >&2
    exit 1
fi
echo '5e3a7beac3c5b7a5d766c30ff024509bc1df23b3f7c5d74349cc54ed6bc960ef  '"$scanner_dir/known_malicious_hashes.json" | sudo sha256sum -c - >/dev/null
echo '39d74eb091cbe986073f3a711c2f25f3be6975dbb38e2083e5fe5e445e7d4ade  '"$scanner_dir/osv_snapshot.json" | sudo sha256sum -c - >/dev/null
echo '217b121bc5943e61d483de8f15d89be9bb91b7b1d3e18d5f18b8396f59f2683e  '"$scanner_dir/rules/supply_chain.yar" | sudo sha256sum -c - >/dev/null
if sudo docker container inspect "$container" >/dev/null 2>&1 ||
   sudo test -e "$data_dir" || sudo test -e "$env_path"; then
    echo 'r7 container or run state already exists; preserve it and inspect before continuing' >&2
    exit 1
fi

sudo install -d -m 0700 -o 999 -g 999 "$data_dir/p2p"
sudo chown 999:999 "$data_dir"
sudo install -m 0600 -o root -g root "$stage/node.env" "$env_path"
sudo install -m 0600 -o 999 -g 999 "$stage/identity.key" "$data_dir/p2p/identity.key"
rm -f "$stage/node.env" "$stage/identity.key"
rmdir "$stage" 2>/dev/null || true

args=(
    -d --name "$container" --restart unless-stopped
    --env-file "$env_path"
    -p "$host_api_port:8080" -p "$host_p2p_port:9000"
    --mount "type=bind,source=$data_dir,target=/data"
    --mount "type=bind,source=$scanner_dir,target=/scanner,readonly"
)
if [ "$role" = validator ]; then
    docker_group=$(stat -c '%g' /var/run/docker.sock)
    args+=(--privileged --group-add "$docker_group" -v /var/run/docker.sock:/var/run/docker.sock)
fi
sudo docker run "${args[@]}" "$image" >/dev/null
actual_image_id=$(sudo docker inspect "$container" --format '{{.Image}}')
if [ "$actual_image_id" != "$expected_index_id" ] && [ "$actual_image_id" != "$expected_config_id" ]; then
    echo 'Started r7 node image does not match the reviewed r9 config digest' >&2
    exit 1
fi
echo "Started isolated r7 $slot ($role) with pinned scanner assets and image $actual_image_id"
