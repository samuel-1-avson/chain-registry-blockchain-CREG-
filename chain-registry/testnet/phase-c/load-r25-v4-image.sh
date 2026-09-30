#!/usr/bin/env bash
# The V4 VM has no cross-project Artifact Registry credential. Verify the
# locally exported immutable image archive before loading the same image.
set -euo pipefail
archive=/tmp/creg-node-r25-full.tar.gz
image=creg-node:phase-c-r25-follower-capture
printf '%s  %s\n' \
  06f96d90ba84784500978690a5ffb8a50432c4c7600befc42e0be3888db3377d \
  "$archive" | sha256sum -c -
sudo docker load -i "$archive" >/dev/null
[[ $(sudo docker image inspect -f '{{.Id}}' "$image") == \
  sha256:704cd7dfb89a048c146a3de502416c31a6f77c0e3f493a98f4fe3098c732046c ]]
[[ $(sudo docker run --rm --entrypoint sha256sum "$image" /app/creg-node | cut -d' ' -f1) == \
  24925bf0df62ed9fafe4565f9969ba83c0beefbbd6725abbd8814f99ea04cf07 ]]
printf '%s\n' 'R25 image archive, image ID, and node binary hashes verified on real V4 host'
