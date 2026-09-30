#!/usr/bin/env python3
"""Remove the completed h16 capture arm and retain both R25 containers."""

import datetime as dt
import hashlib
import json
import os
import pathlib
import re
import subprocess
import sys
import time
import urllib.parse
import urllib.request

CONTAINER = "creg-phase-c-r7-v4"
IMAGE_ID = "sha256:704cd7dfb89a048c146a3de502416c31a6f77c0e3f493a98f4fe3098c732046c"
CHAIN_ID = "creg-phase-c-testnet-2026-09-26-r7"
SPEC_GENESIS = "1852b9a86dd3b53e7b2de4a774723a4fe48e862f9c61d14e5917b42b21055234"
BLOCK_ZERO_HASH = "0f2a3b109318f0979807f3d921f72775b2a37ce4eb28415a62d9f53b3766eed4"
PROFILE = "e067b8fc4aee823203a8a8204d2b57a4a097798d739d338bf8a45b4ff7b5ceb2"
CANONICAL = "npm:escape-string-regexp@5.0.0"
HEIGHT = 16
BLOCK_HASH = "4bd594d71b4fc8bc785c34b1e46291518328233c8aed616171efd9add073e260"
BASE = pathlib.Path("/var/lib/creg/phase-c-r7")
DATA = BASE / "data"
SCANNER = BASE / "scanner-r12-osv-escape-live"
ENV_FILE = BASE / "node.env"
EVIDENCE = BASE / "evidence"
ANSI = re.compile(r"\x1b\[[0-?]*[ -/]*[@-~]")


def utc_now():
    return dt.datetime.now(dt.timezone.utc).isoformat()


def run(args, *, check=True):
    return subprocess.run(args, check=check, capture_output=True, text=True)


def get_json(path):
    with urllib.request.urlopen("http://127.0.0.1:8080" + path, timeout=10) as response:
        return json.loads(response.read().decode("utf-8"))


def get_text(path):
    with urllib.request.urlopen("http://127.0.0.1:8080" + path, timeout=10) as response:
        return response.read().decode("utf-8")


def env_map(path):
    result = {}
    for line in path.read_text(encoding="utf-8").splitlines():
        if "=" in line and not line.lstrip().startswith("#"):
            key, value = line.split("=", 1)
            result[key] = value
    return result


def inspect(name=CONTAINER):
    return json.loads(run(["docker", "inspect", name]).stdout)[0]


def metrics_value(metrics, name):
    match = re.search(r"^" + re.escape(name) + r"\s+([-+]?[0-9.]+)\s*$", metrics, re.M)
    return float(match.group(1)) if match else None


def save(path, report):
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    os.chmod(temporary, 0o600)
    os.replace(temporary, path)
    os.chmod(path, 0o600)


def preflight():
    info = inspect()
    container_env = {}
    for entry in info.get("Config", {}).get("Env", []):
        if "=" in entry:
            key, value = entry.split("=", 1)
            container_env[key] = value
    stats = get_json("/v1/public/chain/stats")
    health = get_json("/v1/public/health")
    pbft = get_json("/v1/consensus/pbft")
    package = get_json("/v1/public/packages/" + urllib.parse.quote(CANONICAL, safe=""))
    metrics = get_text("/metrics")
    expected_arm = {
        "CREG_PHASE_C_COMMIT_CAPTURE_NODE_ID": "validator-4",
        "CREG_PHASE_C_COMMIT_CAPTURE_CHAIN_ID": CHAIN_ID,
        "CREG_PHASE_C_COMMIT_CAPTURE_GENESIS_HASH": SPEC_GENESIS,
        "CREG_PHASE_C_COMMIT_CAPTURE_HEIGHT": str(HEIGHT),
        "CREG_PHASE_C_COMMIT_CAPTURE_PACKAGE_CANONICAL": CANONICAL,
        "CREG_PHASE_C_COMMIT_CAPTURE_PAUSE_MS": "30000",
    }
    checks = {
        "armed_r25_v4_running": info.get("Image") == IMAGE_ID and info.get("State", {}).get("Running") is True,
        "exact_capture_arm_present": all(container_env.get(key) == value for key, value in expected_arm.items()),
        "r7_h16_shared_tip": stats.get("chain_id") == CHAIN_ID and stats.get("genesis_hash", "").lower().removeprefix("0x") == BLOCK_ZERO_HASH and stats.get("tip_height") == HEIGHT and stats.get("finalized_height") == HEIGHT and stats.get("tip_hash", "").lower().removeprefix("0x") == BLOCK_HASH and stats.get("spec_signature_verified") is True,
        "four_validator_roster": stats.get("validator_count") == 4 and stats.get("active_validators") == 4,
        "package_revoked": package.get("status", "").lower() == "revoked" and package.get("block_phase") == "committed",
        "v4_healthy": health.get("status") == "ok" and health.get("node_id") == "validator-4" and health.get("is_validator") is True,
        "pbft_clear": pbft.get("active_round_count") == 0 and not pbft.get("rounds"),
        "r12_profile_ready": f'profile_digest="{PROFILE}"' in metrics and metrics_value(metrics, "creg_scanner_profile_ready") == 1 and metrics_value(metrics, "creg_scanner_osv_ready") == 1,
        "queues_clear": metrics_value(metrics, "creg_pending_block_txs") == 0 and metrics_value(metrics, "creg_durable_finalized_work_depth") == 0 and metrics_value(metrics, "creg_forced_inclusion_pending") == 0,
        "identity_and_signer_journal_present": (DATA / "p2p" / "identity.key").is_file() and (DATA / "pbft-safety.rocksdb").is_dir(),
    }
    if not all(checks.values()):
        raise RuntimeError("V4 disarm preflight failed: " + json.dumps({key: value for key, value in checks.items() if not value}))
    return info, stats, health, package, pbft, metrics, container_env, checks


def main():
    if os.geteuid() != 0:
        raise SystemExit("Run this disarm helper as root on the V4 host.")
    if not ENV_FILE.is_file() or not SCANNER.is_dir():
        raise SystemExit("The unarmed node.env or R12 scanner directory is missing.")
    host_env = env_map(ENV_FILE)
    if any(key.startswith("CREG_PHASE_C_COMMIT_CAPTURE_") for key in host_env):
        raise SystemExit("The canonical node.env unexpectedly contains a capture arm; refusing to overwrite it.")
    stamp = dt.datetime.now(dt.timezone.utc).strftime("%Y%m%dT%H%M%S") + f"-{os.getpid()}Z"
    preserved = f"{CONTAINER}-w3-h16-arm-preserved-{stamp}"
    report_path = EVIDENCE / f"w3-v4-r25-disarm-h16-{stamp}.json"
    if report_path.exists() or subprocess.run(["docker", "container", "inspect", preserved], capture_output=True).returncode == 0:
        raise SystemExit("A unique disarm artifact already exists; refusing to overwrite it.")

    before_info, before_stats, before_health, before_package, before_pbft, _, before_env, checks = preflight()
    identity = DATA / "p2p" / "identity.key"
    identity_hash = hashlib.sha256(identity.read_bytes()).hexdigest()
    prior_name = next((key for key in before_env if key.startswith("CREG_PHASE_C_COMMIT_CAPTURE_")), None)
    report = {
        "schema": "creg-r25-v4-durable-commit-disarm-v1",
        "captured_at_utc": utc_now(), "node_id": "validator-4", "chain_id": CHAIN_ID,
        "signed_spec_genesis_identity": SPEC_GENESIS, "height": HEIGHT, "canonical": CANONICAL,
        "block_hash": BLOCK_HASH, "armed_container_id": before_info.get("Id"),
        "image_id": before_info.get("Image"), "preserved_armed_container": preserved,
        "identity_sha256_before": identity_hash, "preflight": checks,
        "pre_disarm_stats": before_stats, "pre_disarm_health": before_health,
        "pre_disarm_package_status": before_package.get("status"), "pre_disarm_pbft": before_pbft,
        "hook_log_lines": [], "controlled_crash_performed": False,
        "replay_observed": False, "result": "disarming_after_pause_missed_by_monitor",
    }
    EVIDENCE.mkdir(mode=0o700, parents=True, exist_ok=True)
    save(report_path, report)

    run(["docker", "stop", "--time", "30", CONTAINER])
    run(["docker", "rename", CONTAINER, preserved])
    docker_gid = str(pathlib.Path("/var/run/docker.sock").stat().st_gid)
    try:
        new_id = run([
            "docker", "run", "-d", "--name", CONTAINER, "--restart", "unless-stopped",
            "--env-file", str(ENV_FILE), "-p", "8080:8080", "-p", "9000:9000",
            "--mount", f"type=bind,source={DATA},target=/data",
            "--mount", f"type=bind,source={SCANNER},target=/scanner,readonly",
            "--privileged", "--group-add", docker_gid,
            "--mount", "type=bind,source=/var/run/docker.sock,target=/var/run/docker.sock",
            "--security-opt", "label=disable", IMAGE_ID,
        ])
    except Exception:
        if subprocess.run(["docker", "container", "inspect", CONTAINER], capture_output=True).returncode != 0:
            run(["docker", "rename", preserved, CONTAINER])
            run(["docker", "start", CONTAINER])
        raise

    deadline = time.monotonic() + 360
    last_error = None
    while time.monotonic() < deadline:
        try:
            info = inspect()
            stats = get_json("/v1/public/chain/stats")
            health = get_json("/v1/public/health")
            pbft = get_json("/v1/consensus/pbft")
            package = get_json("/v1/public/packages/" + urllib.parse.quote(CANONICAL, safe=""))
            metrics = get_text("/metrics")
            env = {}
            for entry in info.get("Config", {}).get("Env", []):
                if "=" in entry:
                    key, value = entry.split("=", 1)
                    env[key] = value
            mounts = info.get("Mounts", [])
            post_checks = {
                "new_r25_container_running_healthy": info.get("Image") == IMAGE_ID and info.get("State", {}).get("Running") is True and info.get("State", {}).get("Health", {}).get("Status") in ("healthy", None),
                "capture_arm_unset": not any(key.startswith("CREG_PHASE_C_COMMIT_CAPTURE_") for key in env),
                "same_data_and_read_only_scanner": any(m.get("Source") == str(DATA) and m.get("Destination") == "/data" and m.get("RW") is True for m in mounts) and any(m.get("Source") == str(SCANNER) and m.get("Destination") == "/scanner" and m.get("RW") is False for m in mounts),
                "same_h16_canonical_tip": stats.get("chain_id") == CHAIN_ID and stats.get("genesis_hash", "").lower().removeprefix("0x") == BLOCK_ZERO_HASH and stats.get("tip_height") == HEIGHT and stats.get("finalized_height") == HEIGHT and stats.get("tip_hash", "").lower().removeprefix("0x") == BLOCK_HASH and stats.get("spec_signature_verified") is True,
                "v4_healthy_and_peered": health.get("status") == "ok" and health.get("node_id") == "validator-4" and health.get("is_validator") is True and stats.get("peer_count", 0) > 0,
                "package_remains_revoked": package.get("status", "").lower() == "revoked" and package.get("block_phase") == "committed",
                "pbft_clear": pbft.get("active_round_count") == 0 and not pbft.get("rounds"),
                "r12_profile_ready": f'profile_digest="{PROFILE}"' in metrics and metrics_value(metrics, "creg_scanner_profile_ready") == 1 and metrics_value(metrics, "creg_scanner_osv_ready") == 1,
                "queues_clear": metrics_value(metrics, "creg_pending_block_txs") == 0 and metrics_value(metrics, "creg_durable_finalized_work_depth") == 0 and metrics_value(metrics, "creg_forced_inclusion_pending") == 0,
                "identity_unchanged_and_journal_present": identity.is_file() and hashlib.sha256(identity.read_bytes()).hexdigest() == identity_hash and (DATA / "pbft-safety.rocksdb").is_dir(),
            }
            if all(post_checks.values()):
                logs = run(["docker", "logs", "--since", "2026-09-28T01:10:55Z", "--timestamps", preserved], check=False).stdout
                markers = [ANSI.sub("", line).strip() for line in logs.splitlines() if "pbft_local_commit_durable" in line or "phase_c_durable_commit_capture_paused" in line or "phase_c_durable_commit_capture_resumed" in line]
                report.update({
                    "captured_at_utc": utc_now(), "unarmed_container_id": info.get("Id"),
                    "post_disarm_stats": stats, "post_disarm_health": health,
                    "post_disarm_pbft": pbft, "post_disarm_package_status": package.get("status"),
                    "identity_sha256_after": hashlib.sha256(identity.read_bytes()).hexdigest(),
                    "post_disarm_checks": post_checks, "hook_log_lines": markers,
                    "durable_commit_marker_seen": any("pbft_local_commit_durable" in line for line in markers),
                    "pause_marker_seen": any("phase_c_durable_commit_capture_paused" in line for line in markers),
                    "pause_elapsed_normally": any("phase_c_durable_commit_capture_resumed" in line for line in markers),
                    "controlled_crash_performed": False, "replay_observed": False,
                    "result": "disarmed_h16_shared_fleet_healthy_replay_unproven",
                })
                save(report_path, report)
                print(json.dumps(report, indent=2), flush=True)
                return 0
            last_error = json.dumps({key: value for key, value in post_checks.items() if not value})
        except Exception as error:
            last_error = str(error)
        time.sleep(2)

    report["result"] = "disarm_postflight_incomplete"
    report["post_disarm_error"] = last_error
    save(report_path, report)
    print(json.dumps(report, indent=2), flush=True)
    raise SystemExit("The unarmed V4 postflight did not pass; inspect the saved report and keep the other validators online.")


if __name__ == "__main__":
    try:
        sys.exit(main())
    except Exception as error:
        print(str(error), file=sys.stderr, flush=True)
        sys.exit(1)
