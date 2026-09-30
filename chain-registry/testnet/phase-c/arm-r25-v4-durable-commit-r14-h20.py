#!/usr/bin/env python3
"""Guarded R25/R14 V4 arm for the real h20 revoke recovery capture."""

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
BINARY_SHA = "24925bf0df62ed9fafe4565f9969ba83c0beefbbd6725abbd8814f99ea04cf07"
CHAIN_ID = "creg-phase-c-testnet-2026-09-26-r7"
SPEC_GENESIS = "1852b9a86dd3b53e7b2de4a774723a4fe48e862f9c61d14e5917b42b21055234"
NETWORK_GENESIS = "0f2a3b109318f0979807f3d921f72775b2a37ce4eb28415a62d9f53b3766eed4"
PROFILE = "7e1d95f9c8423778211944a1dea46674d8cbd3f0aee3b0eea6b5250f11d009cd"
CANONICAL = "npm:ansi-regex@6.1.0"
HEIGHT = 20
SHARED_HEIGHT = 19
SHARED_HASH = "e31b9e7440523d31a66b468e53115ec7cfbbd370e42dd8b1d9439d9a50eeea8b"
BASE = pathlib.Path("/var/lib/creg/phase-c-r7")
DATA = BASE / "data"
SCANNER = BASE / "scanner-r14-ansi-regex-live"
ENV_FILE = BASE / "node.env"
EVIDENCE = BASE / "evidence"


def utc_now():
    return dt.datetime.now(dt.timezone.utc).isoformat()


def run(args, *, check=True):
    result = subprocess.run(args, check=check, capture_output=True, text=True)
    return result.stdout.strip()


def read_json(url_path):
    with urllib.request.urlopen("http://127.0.0.1:8080" + url_path, timeout=10) as response:
        return json.loads(response.read().decode("utf-8"))


def read_text(url_path):
    with urllib.request.urlopen("http://127.0.0.1:8080" + url_path, timeout=10) as response:
        return response.read().decode("utf-8")


def parse_env(path):
    result = {}
    for line in path.read_text(encoding="utf-8").splitlines():
        if "=" in line and not line.lstrip().startswith("#"):
            key, value = line.split("=", 1)
            result[key] = value
    return result


def container_info():
    return json.loads(run(["docker", "inspect", CONTAINER]))[0]


def metrics_value(metrics, name):
    match = re.search(r"^" + re.escape(name) + r"\s+([-+]?[0-9.]+)\s*$", metrics, re.M)
    return float(match.group(1)) if match else None


def assert_preflight():
    if os.geteuid() != 0:
        raise RuntimeError("Run this arm helper as root on the V4 host.")
    if not (DATA / "chain.rocksdb").is_dir() or not (DATA / "pbft-safety.rocksdb").is_dir():
        raise RuntimeError("V4 chain store or durable PBFT signer journal is missing.")
    identity = DATA / "p2p" / "identity.key"
    if not identity.is_file() or not SCANNER.is_dir() or not ENV_FILE.is_file():
        raise RuntimeError("V4 identity, R14 scanner bundle, or node environment is missing.")

    info = container_info()
    env_file = parse_env(ENV_FILE)
    container_env = {}
    for item in info.get("Config", {}).get("Env", []):
        if "=" in item:
            key, value = item.split("=", 1)
            container_env[key] = value
    mounts = info.get("Mounts", [])
    stats = read_json("/v1/public/chain/stats")
    health = read_json("/v1/public/health")
    pbft = read_json("/v1/consensus/pbft")
    package = read_json("/v1/public/packages/" + urllib.parse.quote(CANONICAL, safe=""))
    metrics = read_text("/metrics")
    binary_sha = run(["docker", "exec", CONTAINER, "sha256sum", "/app/creg-node"]).split()[0]
    capture_keys = [key for key in env_file if key.startswith("CREG_PHASE_C_COMMIT_CAPTURE_")]

    checks = {
        "current_r25_container_running": info.get("Image") == IMAGE_ID and info.get("State", {}).get("Running") is True,
        "current_binary_matches_build": binary_sha == BINARY_SHA,
        "container_env_matches_node_env": all(container_env.get(key) == value for key, value in env_file.items()),
        "capture_arm_unset": not capture_keys and not any(key.startswith("CREG_PHASE_C_COMMIT_CAPTURE_") for key in container_env),
        "v4_identity_and_quorum_config": env_file.get("CREG_NODE_ID") == "validator-4" and env_file.get("CREG_NODE_ROLE") == "validator" and env_file.get("CREG_PBFT_ALLOW_SMALL_CLUSTER_QUORUM", "").lower() == "false",
        "r7_signed_chain_config": env_file.get("CREG_CHAIN_ID") == CHAIN_ID and env_file.get("CREG_GENESIS_HASH", "").lower().removeprefix("0x") == SPEC_GENESIS,
        "same_data_and_read_only_r14_mounts": any(m.get("Source") == str(DATA) and m.get("Destination") == "/data" and m.get("RW") is True for m in mounts) and any(m.get("Source") == str(SCANNER) and m.get("Destination") == "/scanner" and m.get("RW") is False for m in mounts),
        "docker_socket_mount_retained": any(m.get("Source") == "/var/run/docker.sock" and m.get("Destination") == "/var/run/docker.sock" and m.get("RW") is True for m in mounts),
        "shared_h19_tip": stats.get("chain_id") == CHAIN_ID and stats.get("genesis_hash", "").lower().removeprefix("0x") == NETWORK_GENESIS and stats.get("tip_height") == SHARED_HEIGHT and stats.get("finalized_height") == SHARED_HEIGHT and stats.get("tip_hash", "").lower().removeprefix("0x") == SHARED_HASH and stats.get("spec_signature_verified") is True,
        "four_validator_roster_and_peers": stats.get("validator_count") == 4 and stats.get("active_validators") == 4 and stats.get("peer_count", 0) > 0,
        "v4_health": health.get("status") == "ok" and health.get("node_id") == "validator-4" and health.get("is_validator") is True,
        "recovery_rounds_clear": pbft.get("active_round_count") == 0 and not pbft.get("rounds"),
        "package_verified_before_revoke": package.get("status", "").lower() == "verified" and package.get("block_phase") == "committed",
        "r14_profile_ready": f'profile_digest="{PROFILE}"' in metrics and metrics_value(metrics, "creg_scanner_profile_ready") == 1 and metrics_value(metrics, "creg_scanner_osv_ready") == 1,
        "queues_clear": metrics_value(metrics, "creg_pending_block_txs") == 0 and metrics_value(metrics, "creg_durable_finalized_work_depth") == 0 and metrics_value(metrics, "creg_forced_inclusion_pending") == 0,
    }
    if not all(checks.values()):
        raise RuntimeError("V4 arm preflight failed: " + json.dumps({key: value for key, value in checks.items() if not value}))
    return info, env_file, stats, health, package, checks, hashlib.sha256(identity.read_bytes()).hexdigest(), binary_sha


def save_report(path, report):
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    os.chmod(temporary, 0o600)
    os.replace(temporary, path)
    os.chmod(path, 0o600)


def main():
    EVIDENCE.mkdir(mode=0o700, parents=True, exist_ok=True)
    stamp = dt.datetime.now(dt.timezone.utc).strftime("%Y%m%dT%H%M%S") + f"-{os.getpid()}Z"
    report_path = EVIDENCE / f"w3-v4-r25-arm-{stamp}.json"
    armed_env_path = BASE / f"node.env.w3-r25-capture-{stamp}"
    preserved_name = f"{CONTAINER}-r25-pre-w3-{stamp}"
    if report_path.exists() or armed_env_path.exists():
        raise RuntimeError("A W3 artifact with this unique run name already exists.")
    existing_container = subprocess.run(["docker", "container", "inspect", preserved_name], capture_output=True, text=True)
    if existing_container.returncode == 0:
        raise RuntimeError("Preserved-container name already exists; refusing to overwrite it.")

    info, env_file, stats, health, package, checks, identity_sha, binary_sha = assert_preflight()
    source = ENV_FILE.read_bytes()
    additions = {
        "CREG_PHASE_C_COMMIT_CAPTURE_NODE_ID": "validator-4",
        "CREG_PHASE_C_COMMIT_CAPTURE_CHAIN_ID": CHAIN_ID,
        "CREG_PHASE_C_COMMIT_CAPTURE_GENESIS_HASH": SPEC_GENESIS,
        "CREG_PHASE_C_COMMIT_CAPTURE_HEIGHT": str(HEIGHT),
        "CREG_PHASE_C_COMMIT_CAPTURE_PACKAGE_CANONICAL": CANONICAL,
        "CREG_PHASE_C_COMMIT_CAPTURE_PAUSE_MS": "30000",
    }
    payload = source + (b"" if source.endswith(b"\n") else b"\n")
    payload += "".join(f"{key}={value}\n" for key, value in additions.items()).encode()
    fd = os.open(armed_env_path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    try:
        os.write(fd, payload)
        os.fsync(fd)
    finally:
        os.close(fd)

    report = {
        "schema": "creg-r25-v4-durable-commit-arm-v1",
        "armed_at_utc": utc_now(),
        "node_id": "validator-4",
        "chain_id": CHAIN_ID,
        "signed_spec_genesis_identity": SPEC_GENESIS,
        "runtime_network_genesis_hash": NETWORK_GENESIS,
        "height": HEIGHT,
        "canonical": CANONICAL,
        "binary_sha256": binary_sha,
        "image_id": info.get("Image"),
        "prior_container_id": info.get("Id"),
        "preserved_prior_container": preserved_name,
        "current_container_id": None,
        "identity_sha256": identity_sha,
        "pre_arm_stats": stats,
        "pre_arm_health": health,
        "pre_arm_package_status": package.get("status"),
        "capture_environment_file": str(armed_env_path),
        "checks": checks,
        "result": "arming",
    }
    save_report(report_path, report)

    run(["docker", "stop", "--time", "30", CONTAINER])
    if run(["docker", "inspect", "--format", "{{.State.Status}}", CONTAINER]) != "exited":
        raise RuntimeError("The old R25 V4 container did not stop cleanly; its name and data are preserved.")
    run(["docker", "rename", CONTAINER, preserved_name])
    docker_gid = str(pathlib.Path("/var/run/docker.sock").stat().st_gid)
    try:
        run([
            "docker", "run", "-d", "--name", CONTAINER, "--restart", "unless-stopped",
            "--env-file", str(armed_env_path), "-p", "8080:8080", "-p", "9000:9000",
            "--mount", f"type=bind,source={DATA},target=/data",
            "--mount", f"type=bind,source={SCANNER},target=/scanner,readonly",
            "--privileged", "--group-add", docker_gid,
            "--mount", "type=bind,source=/var/run/docker.sock,target=/var/run/docker.sock",
            "--security-opt", "label=disable", IMAGE_ID,
        ])
    except Exception:
        if not run(["docker", "container", "inspect", CONTAINER], check=False):
            run(["docker", "rename", preserved_name, CONTAINER])
            run(["docker", "start", CONTAINER])
        raise

    deadline = time.monotonic() + 360
    last_error = None
    armed_info = None
    while time.monotonic() < deadline:
        try:
            armed_info, _, armed_stats, armed_health, armed_package, armed_checks, armed_identity, armed_binary = assert_preflight_armed()
            if all(armed_checks.values()):
                report.update({
                    "current_container_id": armed_info.get("Id"),
                    "restart_policy": armed_info.get("HostConfig", {}).get("RestartPolicy", {}).get("Name"),
                    "post_arm_stats": armed_stats,
                    "post_arm_health": armed_health,
                    "post_arm_package_status": armed_package.get("status"),
                    "post_arm_identity_sha256": armed_identity,
                    "post_arm_binary_sha256": armed_binary,
                    "post_arm_checks": armed_checks,
                    "result": "armed_and_ready_for_revoke_capture",
                })
                save_report(report_path, report)
                print(json.dumps(report, indent=2), flush=True)
                return 0
        except Exception as error:
            last_error = str(error)
        time.sleep(2)

    report["result"] = "armed_container_started_but_postflight_failed"
    report["post_arm_error"] = last_error
    report["current_container_id"] = armed_info.get("Id") if armed_info else None
    save_report(report_path, report)
    print(json.dumps(report, indent=2), flush=True)
    raise RuntimeError("R25 V4 arm postflight failed. Leave the three other validators online and inspect the preserved containers and report before further action.")


def assert_preflight_armed():
    info = container_info()
    env = {}
    for entry in info.get("Config", {}).get("Env", []):
        if "=" in entry:
            key, value = entry.split("=", 1)
            env[key] = value
    stats = read_json("/v1/public/chain/stats")
    health = read_json("/v1/public/health")
    pbft = read_json("/v1/consensus/pbft")
    package = read_json("/v1/public/packages/" + urllib.parse.quote(CANONICAL, safe=""))
    metrics = read_text("/metrics")
    binary_sha = run(["docker", "exec", CONTAINER, "sha256sum", "/app/creg-node"]).split()[0]
    identity = DATA / "p2p" / "identity.key"
    mounts = info.get("Mounts", [])
    expected_arm = {
        "CREG_PHASE_C_COMMIT_CAPTURE_NODE_ID": "validator-4",
        "CREG_PHASE_C_COMMIT_CAPTURE_CHAIN_ID": CHAIN_ID,
        "CREG_PHASE_C_COMMIT_CAPTURE_GENESIS_HASH": SPEC_GENESIS,
        "CREG_PHASE_C_COMMIT_CAPTURE_HEIGHT": str(HEIGHT),
        "CREG_PHASE_C_COMMIT_CAPTURE_PACKAGE_CANONICAL": CANONICAL,
        "CREG_PHASE_C_COMMIT_CAPTURE_PAUSE_MS": "30000",
    }
    checks = {
        "armed_r25_container_running": info.get("Image") == IMAGE_ID and info.get("State", {}).get("Running") is True,
        "health_check_passed": info.get("State", {}).get("Health", {}).get("Status") in ("healthy", None),
        "same_binary": binary_sha == BINARY_SHA,
        "exact_capture_arm": all(env.get(key) == value for key, value in expected_arm.items()),
        "same_chain_and_identity": env.get("CREG_NODE_ID") == "validator-4" and env.get("CREG_CHAIN_ID") == CHAIN_ID and env.get("CREG_GENESIS_HASH", "").lower().removeprefix("0x") == SPEC_GENESIS and stats.get("genesis_hash", "").lower().removeprefix("0x") == NETWORK_GENESIS,
        "same_data_and_scanner_mounts": any(m.get("Source") == str(DATA) and m.get("Destination") == "/data" and m.get("RW") is True for m in mounts) and any(m.get("Source") == str(SCANNER) and m.get("Destination") == "/scanner" and m.get("RW") is False for m in mounts),
        "same_h19_tip": stats.get("tip_height") == SHARED_HEIGHT and stats.get("finalized_height") == SHARED_HEIGHT and stats.get("tip_hash", "").lower().removeprefix("0x") == SHARED_HASH and stats.get("spec_signature_verified") is True,
        "four_validator_roster": stats.get("validator_count") == 4 and stats.get("active_validators") == 4,
        "v4_health": health.get("status") == "ok" and health.get("node_id") == "validator-4" and health.get("is_validator") is True,
        "pbft_clear": pbft.get("active_round_count") == 0 and not pbft.get("rounds"),
        "package_still_verified": package.get("status", "").lower() == "verified" and package.get("block_phase") == "committed",
        "r14_profile_ready": f'profile_digest="{PROFILE}"' in metrics and metrics_value(metrics, "creg_scanner_profile_ready") == 1 and metrics_value(metrics, "creg_scanner_osv_ready") == 1,
        "queues_clear": metrics_value(metrics, "creg_pending_block_txs") == 0 and metrics_value(metrics, "creg_durable_finalized_work_depth") == 0 and metrics_value(metrics, "creg_forced_inclusion_pending") == 0,
        "identity_and_journal_present": identity.is_file() and (DATA / "pbft-safety.rocksdb").is_dir(),
    }
    if info.get("State", {}).get("Health", {}).get("Status") == "unhealthy":
        checks["health_check_passed"] = False
    return info, env, stats, health, package, checks, hashlib.sha256(identity.read_bytes()).hexdigest(), binary_sha


if __name__ == "__main__":
    try:
        sys.exit(main())
    except Exception as error:
        print(str(error), file=sys.stderr, flush=True)
        sys.exit(1)
