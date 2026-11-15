#!/usr/bin/env bash
# Load the r34 node image on observer-3 from the copied tarball and check it
# against the build before anything uses it. Changes no container.
# Usage: o3-load-r34.sh TAR_SHA256 CONFIG_DIGEST EXPECTED_REVISION EXPECTED_BINARY_SHA256
set -euo pipefail
tar_sha=$1; config=$2; rev=$3; binary=$4
[[ "$tar_sha" =~ ^[0-9a-f]{64}$ && "$config" =~ ^sha256:[0-9a-f]{64}$ && "$rev" =~ ^[0-9a-f]{40}$ && "$binary" =~ ^[0-9a-f]{64}$ ]]
tar=/tmp/creg-node-r34.tar
tag=creg-node:phase-c-equiv-r34-d67e9204e596
[[ $(sha256sum "$tar" | cut -d' ' -f1) == "$tar_sha" ]]
echo "tarball sha256 matches"
sudo docker load -i "$tar"
[[ $(sudo docker image inspect "$tag" --format '{{.Id}}') == "$config" ]]
echo "image id matches the registry config digest"
[[ $(sudo docker image inspect "$tag" --format '{{index .Config.Labels "org.opencontainers.image.revision"}}') == "$rev" ]]
[[ $(sudo docker run --rm --entrypoint /usr/bin/sha256sum "$tag" /app/creg-node | cut -d' ' -f1) == "$binary" ]]
echo "node binary matches the build"
[[ $(sudo docker run --rm "$tag" --version) == 'creg-node 0.1.0' ]]
rm -f -- "$tar"
echo "loaded $tag $config"
