#!/usr/bin/env bash
# Read-only. For every container on this host that carries CREG_VALIDATOR_KEY,
# print its name, state, node id and DERIVED PUBLIC key only.
set -u
sudo python3 - <<'PY'
import json, subprocess
names = subprocess.check_output(['docker', 'ps', '-a', '--format', '{{.Names}}'], text=True).split()
def pub(seed):
    der = bytes.fromhex('302e020100300506032b657004220420') + seed
    return subprocess.run(['openssl', 'pkey', '-inform', 'DER', '-pubout', '-outform', 'DER'],
                          input=der, capture_output=True, check=True).stdout[-32:].hex()
for n in names:
    info = json.loads(subprocess.check_output(['docker', 'inspect', n]))[0]
    env = dict(e.split('=', 1) for e in info['Config']['Env'] if '=' in e)
    raw = (env.get('CREG_VALIDATOR_KEY') or '').strip()
    if not raw:
        continue
    raw = raw[2:] if raw.startswith('0x') else raw
    try:
        p = pub(bytes.fromhex(raw)[:32])
    except Exception as e:
        p = 'error:' + type(e).__name__
    print(f"{n} | {info['State']['Status']} | restart={info['HostConfig']['RestartPolicy']['Name']} | {env.get('CREG_NODE_ID')} | {p}")
PY
