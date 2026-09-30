#!/usr/bin/env python3
"""Crash V4 only at its real R25 durable-COMMIT pause and record replay."""

import datetime as dt
import json
import os
import pathlib
import re
import queue
import subprocess
import sys
import threading
import time
import urllib.parse
import urllib.request

CONTAINER = "creg-phase-c-r7-v4"
IMAGE_ID = "sha256:704cd7dfb89a048c146a3de502416c31a6f77c0e3f493a98f4fe3098c732046c"
CHAIN_ID = "creg-phase-c-testnet-2026-09-26-r7"
SPEC_GENESIS = "1852b9a86dd3b53e7b2de4a774723a4fe48e862f9c61d14e5917b42b21055234"
NETWORK_GENESIS = "0f2a3b109318f0979807f3d921f72775b2a37ce4eb28415a62d9f53b3766eed4"
PROFILE = "8c7f011ab71a9a695ce1364e9958a2906f02645849c3567fc5691206a93c296e"
CANONICAL = "npm:has-flag@5.0.1"
HEIGHT = 18
BASE = pathlib.Path("/var/lib/creg/phase-c-r7")
REPORT = BASE / "evidence" / "w3-v4-durable-commit-capture-r25-r13-h18.json"
FIELD_RE = re.compile(r'(?:^|\s)([a-z_]+)[=:]\s*(?:"([^"]*)"|([^\s,]+))', re.I)
HASH_RE = re.compile(r'block_hash[\s=:\"]+([0-9a-f]{64})', re.I)
ANSI_RE = re.compile(r"\x1b\[[0-?]*[ -/]*[@-~]")
PARENT_HASH = "7c3364858873b00dcb8eef4875c2f1eaa85ade32648850177bd12dcb5f7627d7"


def utc_now():
    return dt.datetime.now(dt.timezone.utc).isoformat()


def run(args, *, check=True):
    return subprocess.run(args, check=check, capture_output=True, text=True)


def inspect():
    return json.loads(run(["docker", "inspect", CONTAINER]).stdout)[0]


def api_json(path):
    with urllib.request.urlopen("http://127.0.0.1:8080" + path, timeout=8) as response:
        return json.loads(response.read().decode("utf-8"))


def api_text(path):
    with urllib.request.urlopen("http://127.0.0.1:8080" + path, timeout=8) as response:
        return response.read().decode("utf-8")


def parse_fields(line):
    hash_match = HASH_RE.search(line)
    fields = {}
    for match in FIELD_RE.finditer(line):
        fields[match.group(1).lower()] = match.group(2) if match.group(2) is not None else match.group(3)
    try:
        height = int(fields["height"].strip('"'))
    except (KeyError, ValueError):
        height = None
    return {"fields": fields, "block_hash": hash_match.group(1).lower() if hash_match else None, "height": height}


def save(report):
    temporary = REPORT.with_suffix(REPORT.suffix + ".tmp")
    temporary.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    os.chmod(temporary, 0o600)
    os.replace(temporary, REPORT)
    os.chmod(REPORT, 0o600)


def metric(metrics, name):
    match = re.search(r"^" + re.escape(name) + r"\s+([-+]?[0-9.]+)\s*$", metrics, re.M)
    return float(match.group(1)) if match else None


def verify_post_replay(block_hash, initial_info):
    deadline = time.monotonic() + 240
    last_error = None
    while time.monotonic() < deadline:
        try:
            info = inspect()
            stats = api_json("/v1/public/chain/stats")
            health = api_json("/v1/public/health")
            pbft = api_json("/v1/consensus/pbft")
            package = api_json("/v1/public/packages/" + urllib.parse.quote(CANONICAL, safe=""))
            metrics = api_text("/metrics")
            started_at = info.get("State", {}).get("StartedAt")
            height_block = api_json(f"/v1/blocks/{HEIGHT}")
            checks = {
                "same_r25_container_restarted": info.get("Id") == initial_info.get("Id") and info.get("Image") == IMAGE_ID and started_at != initial_info.get("State", {}).get("StartedAt") and info.get("State", {}).get("Running") is True,
                "local_h18_matches_durable_commit": stats.get("chain_id") == CHAIN_ID and stats.get("genesis_hash", "").lower().removeprefix("0x") == NETWORK_GENESIS and stats.get("spec_signature_verified") is True and stats.get("tip_height") == HEIGHT and stats.get("finalized_height") == HEIGHT and stats.get("tip_hash", "").lower().removeprefix("0x") == block_hash and height_block.get("hash", "").lower().removeprefix("0x") == block_hash,
                "package_revoked_in_committed_h18": package.get("status", "").lower() == "revoked" and package.get("block_phase") == "committed",
                "v4_healthy": health.get("status") == "ok" and health.get("node_id") == "validator-4" and health.get("is_validator") is True,
                "r13_scanner_ready": f'profile_digest="{PROFILE}"' in metrics and metric(metrics, "creg_scanner_profile_ready") == 1 and metric(metrics, "creg_scanner_osv_ready") == 1,
                "queues_clear": metric(metrics, "creg_pending_block_txs") == 0 and metric(metrics, "creg_durable_finalized_work_depth") == 0 and metric(metrics, "creg_forced_inclusion_pending") == 0,
                "pbft_clear": pbft.get("active_round_count") == 0 and not pbft.get("rounds"),
                "four_validators_active": stats.get("validator_count") == 4 and stats.get("active_validators") == 4,
            }
            if all(checks.values()):
                return {"captured_at_utc": utc_now(), "stats": stats, "health": health, "pbft": pbft,
                        "package_status": package.get("status"), "package_block_hash": package.get("block_hash"),
                        "started_at": started_at, "restart_count": info.get("RestartCount", 0), "checks": checks}
            last_error = json.dumps({key: value for key, value in checks.items() if not value})
        except Exception as error:
            last_error = str(error)
        time.sleep(0.5)
    return {"error": "V4 did not reach the expected local post-replay state in time", "last_error": last_error}


def main():
    global HEIGHT, CANONICAL, REPORT, PARENT_HASH
    if len(sys.argv) == 4:
        HEIGHT = int(sys.argv[1])
        CANONICAL = sys.argv[2]
        PARENT_HASH = sys.argv[3].lower().removeprefix("0x")
        REPORT = BASE / "evidence" / f"w3-v4-durable-commit-capture-r25-r13-h{HEIGHT}.json"
    elif len(sys.argv) != 1:
        raise SystemExit("Usage: capture-r25-v4-durable-commit.py [TARGET_HEIGHT CANONICAL_ID PARENT_BLOCK_HASH]")
    if os.geteuid() != 0:
        raise SystemExit("Run this monitor as root on the V4 host.")
    if REPORT.exists():
        raise SystemExit(f"Evidence already exists; refusing to overwrite {REPORT}.")
    info = inspect()
    env = {}
    for entry in info.get("Config", {}).get("Env", []):
        if "=" in entry:
            key, value = entry.split("=", 1)
            env[key] = value
    expected = {
        "CREG_PHASE_C_COMMIT_CAPTURE_NODE_ID": "validator-4",
        "CREG_PHASE_C_COMMIT_CAPTURE_CHAIN_ID": CHAIN_ID,
        "CREG_PHASE_C_COMMIT_CAPTURE_GENESIS_HASH": SPEC_GENESIS,
        "CREG_PHASE_C_COMMIT_CAPTURE_HEIGHT": str(HEIGHT),
        "CREG_PHASE_C_COMMIT_CAPTURE_PACKAGE_CANONICAL": CANONICAL,
        "CREG_PHASE_C_COMMIT_CAPTURE_PAUSE_MS": "30000",
        "CREG_SCANNER_PROFILE_DIGEST": PROFILE,
    }
    if info.get("Image") != IMAGE_ID or info.get("State", {}).get("Running") is not True:
        raise SystemExit("V4 is not running the reviewed R25 node image.")
    if any(env.get(key) != value for key, value in expected.items()):
        raise SystemExit("V4 does not have the exact R25/R13 durable-COMMIT capture arm.")
    if info.get("HostConfig", {}).get("RestartPolicy", {}).get("Name") != "unless-stopped":
        raise SystemExit("V4 automatic restart policy is not unless-stopped.")
    pre_stats = api_json("/v1/public/chain/stats")
    pre_package = api_json("/v1/public/packages/" + urllib.parse.quote(CANONICAL, safe=""))
    if pre_stats.get("tip_height") != HEIGHT - 1 or pre_stats.get("genesis_hash", "").lower().removeprefix("0x") != NETWORK_GENESIS or pre_stats.get("tip_hash", "").lower().removeprefix("0x") != PARENT_HASH:
        raise SystemExit(f"V4 no longer has the accepted h{HEIGHT - 1} parent tip; refusing the capture.")
    if pre_package.get("status", "").lower() != "verified" or pre_package.get("block_phase") != "committed":
        raise SystemExit("The exact target package is not committed and verified before its revoke.")

    report = {
        "schema": "creg-r25-v4-durable-commit-crash-capture-v1",
        "armed_at_utc": utc_now(), "node_id": "validator-4", "chain_id": CHAIN_ID,
        "signed_spec_genesis_identity": SPEC_GENESIS, "runtime_network_genesis_hash": NETWORK_GENESIS,
        "height": HEIGHT, "canonical": CANONICAL,
        "container_id": info.get("Id"), "image_id": info.get("Image"),
        "initial_restart_count": info.get("RestartCount"), "pre_capture_stats": pre_stats,
        "durable_marker": None, "pause_marker": None, "kill": None,
        "recovery_marker": None, "recovered_tip": None,
        "result": "waiting_for_exact_durable_commit_and_pause_markers",
    }
    save(report)

    logs = subprocess.Popen(["docker", "logs", "--follow", "--tail", "0", "--timestamps", CONTAINER],
                            stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                            text=True, bufsize=1)
    log_lines = queue.Queue()
    def read_logs():
        for observed_line in logs.stdout:
            log_lines.put(observed_line)
    threading.Thread(target=read_logs, daemon=True).start()
    deadline = time.monotonic() + 900
    durable_by_hash = {}
    try:
        while time.monotonic() < deadline and logs.poll() is None:
            try:
                raw_line = log_lines.get(timeout=1)
            except queue.Empty:
                continue
            line = ANSI_RE.sub("", raw_line.rstrip("\r\n"))
            parsed = parse_fields(line)
            values = parsed["fields"]
            common = parsed["height"] == HEIGHT and values.get("chain_id") == CHAIN_ID and values.get("genesis_hash", "").lower().removeprefix("0x") == SPEC_GENESIS
            if "pbft_local_commit_durable" in line and common and values.get("node_id") == "validator-4" and parsed["block_hash"]:
                if "Local PBFT COMMIT durably persisted before consensus application or gossip" in line:
                    durable_by_hash[parsed["block_hash"]] = line
                    report["durable_marker"] = {"observed_at_utc": utc_now(), "block_hash": parsed["block_hash"], "line": line}
                    report["result"] = "durable_commit_seen_waiting_for_exact_pause"
                    save(report)

            if "phase_c_durable_commit_capture_paused" in line and common:
                block_hash = parsed["block_hash"]
                if values.get("node_id") == "validator-4" and values.get("package_canonical") == CANONICAL and block_hash in durable_by_hash and "Durable local COMMIT is held before in-memory application and peer broadcast" in line:
                    pause_time = dt.datetime.fromisoformat(line.split(" ", 1)[0].replace("Z", "+00:00"))
                    pause_age = (dt.datetime.now(dt.timezone.utc) - pause_time).total_seconds()
                    if pause_age >= 20:
                        report["result"] = "missed_pause_window_no_crash_performed"
                        report["pause_age_seconds"] = pause_age
                        save(report)
                        return 1
                    report["pause_marker"] = {"observed_at_utc": utc_now(), "block_hash": block_hash, "line": line}
                    report["durable_marker"]["matching_pause_hash"] = block_hash
                    report["result"] = "pause_seen_sigkill_in_progress"
                    save(report)
                    kill_started = utc_now()
                    killed = run(["docker", "kill", "--signal", "SIGKILL", CONTAINER], check=False)
                    report["kill"] = {"started_at_utc": kill_started, "completed_at_utc": utc_now(), "exit_code": killed.returncode,
                                      "container_id": info.get("Id"), "signal": "SIGKILL", "stdout": killed.stdout.strip(), "stderr": killed.stderr.strip()}
                    if killed.returncode != 0:
                        report["result"] = "sigkill_failed"
                        save(report)
                        raise SystemExit("The exact V4 durable-COMMIT pause was observed but SIGKILL failed; inspect V4 immediately.")
                    started = run(["docker", "start", CONTAINER], check=False)
                    report["manual_start"] = {"started_at_utc": utc_now(), "exit_code": started.returncode,
                                              "stdout": started.stdout.strip(), "stderr": started.stderr.strip()}
                    save(report)
                    if started.returncode != 0:
                        report["result"] = "manual_start_failed"
                        save(report)
                        raise SystemExit("V4 was killed but did not start; inspect V4 immediately.")
                    logs.terminate()
                    try:
                        logs.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        logs.kill()
                        logs.wait(timeout=5)

                    restart_deadline = time.monotonic() + 180
                    recovered_line = None
                    recovered_info = None
                    while time.monotonic() < restart_deadline:
                        try:
                            recovered_info = inspect()
                            if recovered_info.get("Id") == info.get("Id") and recovered_info.get("Image") == IMAGE_ID and recovered_info.get("State", {}).get("StartedAt") != info.get("State", {}).get("StartedAt") and recovered_info.get("State", {}).get("Running") is True:
                                recent = run(["docker", "logs", "--tail", "1500", CONTAINER], check=False).stdout
                                for candidate in reversed(recent.splitlines()):
                                    recovered = parse_fields(candidate)
                                    recovered_fields = recovered["fields"]
                                    recovered_node = recovered_fields.get("validator_id", recovered_fields.get("node_id"))
                                    if "pbft_local_commit_recovered" in candidate and recovered["height"] == HEIGHT and recovered["block_hash"] == block_hash and recovered_node == "validator-4" and recovered_fields.get("chain_id") == CHAIN_ID and recovered_fields.get("genesis_hash", "").lower().removeprefix("0x") == SPEC_GENESIS and "Recovered durable local PBFT COMMIT before P2P and sync services start" in candidate:
                                        recovered_line = candidate
                                        break
                                if recovered_line:
                                    break
                        except Exception:
                            pass
                        time.sleep(0.5)
                    if not recovered_line:
                        report["result"] = "recovery_marker_not_observed"
                        save(report)
                        raise SystemExit("V4 restarted or remains down, but exact local COMMIT replay was not observed.")

                    report["recovery_marker"] = {"observed_at_utc": utc_now(), "block_hash": block_hash, "line": recovered_line}
                    report["restart_count_after_recovery"] = recovered_info.get("RestartCount")
                    report["container_running_after_recovery"] = recovered_info.get("State", {}).get("Running") is True
                    report["result"] = "local_commit_replay_seen_waiting_for_h18_application"
                    save(report)
                    post = verify_post_replay(block_hash, info)
                    report["recovered_tip"] = post
                    if post.get("checks") and all(post["checks"].values()):
                        report["result"] = "passed_local_durable_commit_replay_and_h18_application"
                        save(report)
                        print(json.dumps(report, indent=2), flush=True)
                        return 0
                    report["result"] = "local_commit_replay_seen_but_postflight_incomplete"
                    save(report)
                    print(json.dumps(report, indent=2), flush=True)
                    return 1
        report["result"] = "capture_timeout_no_matching_durable_commit_observed"
        save(report)
        print(json.dumps(report, indent=2), flush=True)
        return 1
    finally:
        if logs.poll() is None:
            logs.terminate()
            try:
                logs.wait(timeout=5)
            except subprocess.TimeoutExpired:
                logs.kill()
                logs.wait(timeout=5)


if __name__ == "__main__":
    sys.exit(main())
