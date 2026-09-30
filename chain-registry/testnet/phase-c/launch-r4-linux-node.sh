#!/usr/bin/env bash
# Launch the expiry repair with a new signed chain identity and fresh storage.
set -euo pipefail
slot=${1:?slot required}
role=${2:?role required}
host_api_port=${3:?host API port required}
host_p2p_port=${4:?host P2P port required}
image=${5:?pinned image required}
genesis_hash=${6:?signed genesis hash required}
index_digest=sha256:ca4d04e77e9680cb878800f2ea72c1ce7dac6bb3575a55c9921a7314975fb1ef
config_digest=sha256:6c6861bc4c8efc6870cea646abc880c383cff1fb57267c699438844611ff543d
container=creg-phase-c-r4-${slot}
base=/var/lib/creg/phase-c-r4
staged=/var/tmp/phase-c-r4-${slot}

case "$slot:$role" in
    v[1-4]:validator|o[1-4]:observer) ;;
    *) echo 'Invalid slot or role' >&2; exit 2 ;;
esac
case "$host_api_port:$host_p2p_port" in
    8080:9000|18080:9001) ;;
    *) echo 'Unexpected port mapping' >&2; exit 2 ;;
esac
[[ "$genesis_hash" =~ ^0x[0-9a-f]{64}$ ]] || exit 2
image_id=$(sudo docker image inspect "$image" --format '{{.Id}}')
if [ "$image_id" != "$index_digest" ] && [ "$image_id" != "$config_digest" ]; then
    echo 'Repair image digest mismatch' >&2; exit 1
fi
if sudo docker inspect "$container" >/dev/null 2>&1 || sudo test -e "$base"; then
    echo 'Fresh run path or container already exists; preserve it' >&2; exit 1
fi
test -f "$staged/node.env" && test -f "$staged/identity.key"
grep -qx 'CREG_CHAIN_ID=creg-phase-c-testnet-2026-09-26' "$staged/node.env"
grep -qx "CREG_GENESIS_HASH=$genesis_hash" "$staged/node.env"
grep -qx 'CREG_CHAIN_SPEC_URL=https://spec.testnet.cregnet.dev/phase-c/r4/chain-spec.json' "$staged/node.env"
grep -qx 'CREG_PBFT_ALLOW_SMALL_CLUSTER_QUORUM=false' "$staged/node.env"
grep -qx 'CREG_DEV_SANDBOX=false' "$staged/node.env"
expected_validator=false
if [ "$role" = validator ]; then expected_validator=true; fi
grep -qx "CREG_IS_VALIDATOR=$expected_validator" "$staged/node.env"

sudo install -d -m 0700 -o 999 -g 999 "$base/data/p2p"
sudo chown 999:999 "$base/data"
sudo chmod 0700 "$base/data"
sudo install -m 0600 -o root -g root "$staged/node.env" "$base/node.env"
sudo install -m 0600 -o 999 -g 999 "$staged/identity.key" "$base/data/p2p/identity.key"
rm -f "$staged/node.env" "$staged/identity.key"
args=(-d --name "$container" --restart unless-stopped --env-file "$base/node.env"
    -p "$host_api_port:8080" -p "$host_p2p_port:9000" -v "$base/data:/data")
if [ "$role" = validator ]; then
    docker_group=$(stat -c '%g' /var/run/docker.sock)
    args+=(--privileged --group-add "$docker_group" -v /var/run/docker.sock:/var/run/docker.sock)
fi
sudo docker run "${args[@]}" "$image" >/dev/null
echo "Started $container with fresh signing storage and image $image_id"
