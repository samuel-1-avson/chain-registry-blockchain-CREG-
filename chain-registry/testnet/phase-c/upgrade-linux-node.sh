#!/usr/bin/env bash
# Replace a stopped candidate container while preserving its data and PBFT journal.
set -euo pipefail

slot=${1:?slot required}
role=${2:?role required}
host_api_port=${3:?host API port required}
host_p2p_port=${4:?host P2P port required}
image=${5:?candidate image required}
expected_index=${6:?pinned OCI digest required}
expected_config=${7:?pinned image config ID required}
container=creg-phase-c-${slot}
old_container=${container}-r1
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
for digest in "$expected_index" "$expected_config"; do
    if ! [[ "$digest" =~ ^sha256:[0-9a-f]{64}$ ]]; then
        echo 'Pinned image identifiers must be SHA-256 digests' >&2
        exit 2
    fi
done
image_id=$(sudo docker image inspect "$image" --format '{{.Id}}')
if [ "$image_id" != "$expected_index" ] && [ "$image_id" != "$expected_config" ]; then
    echo 'Repaired candidate image does not match its pinned artifact' >&2
    exit 1
fi
if ! sudo test -f "$base/node.env" || ! sudo test -f "$data_dir/p2p/identity.key"; then
    echo 'Existing private environment or P2P identity is missing' >&2
    exit 1
fi
if ! sudo grep -qx 'CREG_PBFT_ALLOW_SMALL_CLUSTER_QUORUM=false' "$base/node.env"; then
    echo 'Small-cluster quorum override is forbidden' >&2
    exit 1
fi
if sudo docker container inspect "$old_container" >/dev/null 2>&1; then
    echo "First-run evidence container already exists: $old_container" >&2
    exit 1
fi
state=$(sudo docker inspect "$container" --format '{{.State.Status}}')
if [ "$state" != exited ]; then
    echo "Candidate must be stopped before upgrade; state=$state" >&2
    exit 1
fi

sudo docker rename "$container" "$old_container"
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
if [ "$actual_image_id" != "$expected_index" ] && [ "$actual_image_id" != "$expected_config" ]; then
    echo 'Running candidate image does not match its pinned artifact' >&2
    exit 1
fi
echo "Started repaired $container with preserved PBFT journal; image $actual_image_id"
