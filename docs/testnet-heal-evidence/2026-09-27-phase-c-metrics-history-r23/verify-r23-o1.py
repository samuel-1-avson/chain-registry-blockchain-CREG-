#!/usr/bin/env python3
import datetime
import hashlib
import json
import pathlib
import re
import subprocess
import sys
import time
import urllib.error
import urllib.request

API = "http://127.0.0.1:18080"
CONTAINER = "creg-phase-c-r7-o1"
PRESERVED = "creg-phase-c-r7-o1-r22-preserved-r23-metrics-history-20260927T164425Z"
IMAGE_ID = "sha256:ef21b958b5e3e4f09e1c87290d3d9a50f23b5350d207e5af9982dd1cf5413708"
IMAGE_REF = "us-central1-docker.pkg.dev/gen-lang-client-0022105784/creg-testnet/creg-node:phase-c-secure-r23-metrics-history"
REPO_DIGEST = "us-central1-docker.pkg.dev/gen-lang-client-0022105784/creg-testnet/creg-node@sha256:ef21b958b5e3e4f09e1c87290d3d9a50f23b5350d207e5af9982dd1cf5413708"
OLD_IMAGE_ID = "sha256:2b472a089fe6ac651edfbae51b616456ed335f35992c0374f10c1ea80ce6caa3"
NODE_SHA = "b6cab56f893b13cb0340834d7f69a9c23eb80f12d139186fb1056e5ce2e12972"
CLI_SHA = "b987ba41fe68efde6b33cad2f6a7e8d4784402902094b6b78e0742a19d0705b7"
CHAIN = "creg-phase-c-testnet-2026-09-26-r7"
BLOCK_GENESIS = "0f2a3b109318f0979807f3d921f72775b2a37ce4eb28415a62d9f53b3766eed4"
NETWORK_GENESIS = "1852b9a86dd3b53e7b2de4a774723a4fe48e862f9c61d14e5917b42b21055234"
PROFILE = "f397fac29df29438f0ee15580547def5fe66a03af60153d22993f4a7af9fce8c"
EXPECTED_TIP = "2ece0610b4b808ee5382704b702917dd6f2f3f75d6eeb2a8585aaa9e270507aa"
EXPECTED_HEIGHT = 14


def get_json(path):
    with urllib.request.urlopen(API + path, timeout=12) as response:
        return json.loads(response.read().decode())


def get_text(path):
    with urllib.request.urlopen(API + path, timeout=12) as response:
        return response.read().decode()


def metric_value(metrics, name):
    match = re.search(r"^" + re.escape(name) + r"(?:\{[^}]*\})?\s+([-+]?(?:\d+(?:\.\d*)?|\.\d+)(?:[eE][-+]?\d+)?)$", metrics, re.M)
    return float(match.group(1)) if match else None


last_error = None
for _ in range(180):
    try:
        health = get_json("/v1/public/health")
        stats = get_json("/v1/public/chain/stats")
        if health.get("is_validator") is False and stats.get("peer_count", 0) > 0 and stats.get("active_validators") == 4:
            break
        last_error = {"peer_count": stats.get("peer_count"), "active_validators": stats.get("active_validators")}
    except Exception as error:
        last_error = str(error)
    time.sleep(2)
else:
    raise RuntimeError("O1 did not reach observer peer readiness: " + str(last_error))

metrics = get_text("/metrics")
rounds = get_json("/v1/consensus/pbft").get("rounds", [])
history = get_json("/v1/public/metrics/history?range=5m")
assert stats.get("chain_id") == CHAIN and stats.get("spec_signature_verified") is True
assert stats.get("genesis_hash", "").removeprefix("0x") == BLOCK_GENESIS
assert stats.get("tip_height") == EXPECTED_HEIGHT and stats.get("finalized_height") == EXPECTED_HEIGHT and stats.get("tip_hash") == EXPECTED_TIP
assert stats.get("validator_count") == 4 and stats.get("active_validators") == 4
assert history.get("status") == "ready" and history.get("collector") == "prometheus"
assert history.get("chain_id") == CHAIN and history.get("network_genesis_hash") == NETWORK_GENESIS
samples = history.get("samples", [])
assert len(samples) == history.get("sample_count") and len(samples) >= 3
assert history.get("queried_series", 0) == 7
fields = sorted({key for sample in samples for key in sample if key != "timestamp_ms"})
expected_fields = sorted(("height", "validator_count", "active_validators", "package_count", "pending_tx_count", "publisher_count", "total_stake"))
assert fields == expected_fields
height_samples = [sample for sample in samples if "height" in sample]
assert height_samples and int(float(height_samples[-1]["height"])) == EXPECTED_HEIGHT
latest_sample_ms = int(height_samples[-1]["timestamp_ms"])
assert 0 <= int(time.time() * 1000) - latest_sample_ms <= 120_000
assert f'profile_digest="{PROFILE}"' in metrics
for name in ("creg_scanner_profile_ready", "creg_scanner_osv_ready", "creg_scanner_yara_ready", "creg_scanner_threat_intel_ready"):
    assert metric_value(metrics, name) == 1, name
assert metric_value(metrics, "creg_scanner_threat_intel_entries") == 100
queues = {name: metric_value(metrics, name) for name in ("creg_pending_block_txs", "creg_durable_finalized_work_depth", "creg_durable_finalized_work_ready_depth", "creg_durable_finalized_work_dead_letter_depth", "creg_forced_inclusion_pending")}
assert all(queues[name] == 0 for name in ("creg_pending_block_txs", "creg_durable_finalized_work_depth", "creg_durable_finalized_work_ready_depth", "creg_forced_inclusion_pending"))
assert all(item.get("phase") in ("finalised", "failed") for item in rounds)

query = f'creg_chain_tip_height{{chain_id="{CHAIN}",network_genesis_hash="{NETWORK_GENESIS}",metrics_source="explorer-primary"}}'
raw_prometheus = subprocess.check_output(["docker", "exec", CONTAINER, "curl", "-fsS", "--get", "--data-urlencode", "query=" + query, "http://creg-gcp-prometheus:9090/api/v1/query"], text=True, timeout=12)
prom = json.loads(raw_prometheus)
assert prom.get("status") == "success"
series = prom.get("data", {}).get("result", [])
assert len(series) == 1
labels = series[0].get("metric", {})
assert labels.get("chain_id") == CHAIN and labels.get("network_genesis_hash") == NETWORK_GENESIS and labels.get("metrics_source") == "explorer-primary"
prom_height = int(float(series[0]["value"][1]))
assert prom_height == int(float(height_samples[-1]["height"]))

actual_image = subprocess.check_output(["docker", "inspect", CONTAINER, "--format", "{{.Image}}"], text=True).strip()
assert actual_image == IMAGE_ID
repo_digests = subprocess.check_output(["docker", "image", "inspect", actual_image, "--format", "{{join .RepoDigests \"\\n\"}}"], text=True).splitlines()
assert REPO_DIGEST in repo_digests or ("creg-node@" + REPO_DIGEST.rsplit("@", 1)[-1]) in repo_digests
node_sha = subprocess.check_output(["docker", "exec", CONTAINER, "sha256sum", "/app/creg-node"], text=True).split()[0]
cli_sha = subprocess.check_output(["docker", "exec", CONTAINER, "sha256sum", "/app/creg"], text=True).split()[0]
assert node_sha == NODE_SHA and cli_sha == CLI_SHA
old = json.loads(subprocess.check_output(["docker", "inspect", PRESERVED], text=True))[0]
assert old["Image"] == OLD_IMAGE_ID and old["State"]["Status"] == "exited"
new = json.loads(subprocess.check_output(["docker", "inspect", CONTAINER], text=True))[0]
assert new["State"]["Running"] is True and new["State"].get("Health", {}).get("Status") == "healthy"
networks = sorted(new["NetworkSettings"]["Networks"].keys())
assert {"bridge", "monitoring_default"}.issubset(networks)
mounts = new["Mounts"]
assert any(item["Source"] == "/var/lib/creg/phase-c-r7/data" and item["Destination"] == "/data" and item["RW"] for item in mounts)
assert any(item["Source"] == "/var/lib/creg/phase-c-r7/scanner-r11-osv-kleur" and item["Destination"] == "/scanner" and not item["RW"] for item in mounts)
identity_sha = hashlib.sha256(pathlib.Path("/var/lib/creg/phase-c-r7/data/p2p/identity.key").read_bytes()).hexdigest()
env_sha = hashlib.sha256(pathlib.Path("/var/lib/creg/phase-c-r7/node.env").read_bytes()).hexdigest()
record = {
    "schema": "creg-phase-c-r23-o1-metrics-history-verification-v1",
    "captured_at_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
    "target": {"slot": "o1", "role": "observer", "vm": "creg-testnet-vm", "project": "gen-lang-client-0022105784", "zone": "us-central1-b"},
    "chain": {"chain_id": CHAIN, "block_genesis_hash": BLOCK_GENESIS, "network_genesis_hash": NETWORK_GENESIS, "tip_height": stats["tip_height"], "tip_hash": stats["tip_hash"], "finalized_height": stats["finalized_height"], "signature_verified": stats["spec_signature_verified"], "validators": stats["validator_count"], "active_validators": stats["active_validators"], "peers": stats["peer_count"]},
    "image": {"ref": IMAGE_REF, "repo_digest": REPO_DIGEST, "image_id": actual_image, "previous_image_id": OLD_IMAGE_ID, "node_sha256": node_sha, "cli_sha256": cli_sha},
    "preserved_container": PRESERVED,
    "preserved_r22_state": old["State"]["Status"],
    "data_identity_sha256": identity_sha,
    "node_environment_sha256": env_sha,
    "networks": networks,
    "scanner": {"profile_digest": PROFILE, "ready": True},
    "queues": queues,
    "pbft_clear": True,
    "metrics_history": {"http": 200, "collector": history["collector"], "status": history["status"], "range": history["range"], "sample_count": len(samples), "queried_series": history["queried_series"], "fields": fields, "latest_sample_timestamp_ms": latest_sample_ms, "latest_height": int(float(height_samples[-1]["height"]))},
    "direct_prometheus_cross_check": {"series_count": len(series), "labels": labels, "latest_height": prom_height},
    "public_cutover": False,
    "chain_reset": False,
}
report = pathlib.Path("/var/lib/creg/phase-c-r7/evidence/r23-o1-metrics-history-verification.json")
report.parent.mkdir(parents=True, exist_ok=True)
report.write_text(json.dumps(record, indent=2) + "\n")
print(json.dumps(record, separators=(",", ":")))