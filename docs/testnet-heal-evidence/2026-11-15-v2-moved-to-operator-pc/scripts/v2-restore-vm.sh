#!/usr/bin/env bash
# Put validator-2 back on the VM. Used only if the move fails before v2 starts on the PC.
set -u
sudo rm -f /tmp/v2-move.tar
sudo docker update --restart unless-stopped creg-phase-c-r7-v2 >/dev/null
sudo docker start creg-phase-c-r7-v2 >/dev/null
echo "vm v2: $(sudo docker inspect -f '{{.State.Status}} restart={{.HostConfig.RestartPolicy.Name}}' creg-phase-c-r7-v2)"
