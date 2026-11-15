#!/usr/bin/env bash
# Remove the package (it contains the signing key) from the VM.
set -u
shred -u /tmp/v2-move.tar 2>/dev/null || rm -f /tmp/v2-move.tar
rm -f /tmp/v2-stop-pack.sh
if [[ -e /tmp/v2-move.tar ]]; then echo "package still present"; else echo "package removed"; fi
echo "vm v2: $(sudo docker inspect -f '{{.State.Status}} restart={{.HostConfig.RestartPolicy.Name}}' creg-phase-c-r7-v2)"
