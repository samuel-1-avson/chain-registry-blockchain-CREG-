#!/usr/bin/env bash
# Install a real Phase C candidate on one Linux host without touching the old chain.
set -euo pipefail

slot=${1:?slot required}
role=${2:?role required}
host_api_port=${3:?host API port required}
host_p2p_port=${4:?host P2P port required}
image=${5:?immutable candidate image required}
# Docker Desktop reports the OCI index digest; Linux Docker after `load`
# reports the config digest embedded in that same pinned OCI index.
expected_index_digest=sha256:c1e2aad2571da92da2a4cd78ba6036fcffacc993cafebf46aea877f40f9a29ac
expected_config_id=sha256:9e596fc83aed136253d25706651eaaf355a8002698f125ba189f175bed420da6
container=creg-phase-c-${slot}
base=/var/lib/creg/phase-c
data_dir=$base/data

case "$slot:$role" in
    v[1-4]:validator|o[1-4]:observer) ;;
    *) echo 'Invalid Phase C slot or role' >&2; exit 2 ;;
esac
case "$host_api_port:$host_p2p_port" in
    8080:9000|18080:9001) ;;
    *) echo 'Unexpected candidate port mapping' >&2; exit 2 ;;
esac

image_id=$(sudo docker image inspect "$image" --format '{{.Id}}')
if [ "$image_id" != "$expected_index_digest" ] && [ "$image_id" != "$expected_config_id" ]; then
    echo 'Candidate image digest mismatch' >&2
    exit 1
fi
if sudo docker container inspect "$container" >/dev/null 2>&1; then
    echo "Candidate container already exists: $container" >&2
    exit 1
fi
if [ ! -f /var/tmp/node.env ] || [ ! -f /var/tmp/identity.key ]; then
    echo 'Private node.env and identity.key must be staged before launch' >&2
    exit 1
fi
if ! grep -qx 'CREG_PBFT_ALLOW_SMALL_CLUSTER_QUORUM=false' /var/tmp/node.env; then
    echo 'Small-cluster quorum override is forbidden' >&2
    exit 1
fi
if ! grep -qx 'CREG_CHAIN_ID=creg-phase-c-testnet-2026-09-25' /var/tmp/node.env; then
    echo 'Wrong chain ID in staged environment' >&2
    exit 1
fi

sudo install -d -m 0700 -o 999 -g 999 "$data_dir/p2p"
sudo chown 999:999 "$data_dir"
sudo chmod 0700 "$data_dir"
sudo install -m 0600 -o root -g root /var/tmp/node.env "$base/node.env"
sudo install -m 0600 -o 999 -g 999 /var/tmp/identity.key "$data_dir/p2p/identity.key"
rm -f /var/tmp/node.env /var/tmp/identity.key

args=(
    -d --name "$container" --restart unless-stopped
    --env-file "$base/node.env"
    -p "$host_api_port:8080" -p "$host_p2p_port:9000"
    -v "$data_dir:/data"
)
if [ "$role" = validator ]; then
    docker_group=$(stat -c '%g' /var/run/docker.sock)
    args+=(--privileged --group-add "$docker_group" -v /var/run/docker.sock:/var/run/docker.sock)
fi
sudo docker run "${args[@]}" "$image" >/dev/null
actual_image_id=$(sudo docker inspect "$container" --format '{{.Image}}')
if [ "$actual_image_id" != "$expected_index_digest" ] && [ "$actual_image_id" != "$expected_config_id" ]; then
    echo 'Running candidate image digest mismatch' >&2
    exit 1
fi
echo "Started $container on host API $host_api_port, P2P $host_p2p_port, image $actual_image_id"
