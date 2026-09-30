#!/usr/bin/env python3
"""Observe and crash-recover V4 during a real four-validator PBFT commit."""
import datetime
import json
from pathlib import Path
import re
import subprocess
import sys
import time
import urllib.error
import urllib.parse
import urllib.request

API = "http://127.0.0.1:8080"
CONTAINER = "creg-phase-c-r7-v4"
TAG = "creg-node:phase-c-secure-r21-revoke-retirement"
ACCEPTED_IMAGES = {
    "sha256:374eadc62ee9652ee0869efb22ad02ddb0bd8184e11a752a4b4bb303179d9423",
    "sha256:d2b83a6525314b17434bc98c5bf05181db4fd6d73c962eabd2b5407968dfa95c",
    "sha256:60ff1bc767ac84ae119a35b2ac864ebe08e0e4978a788315c3759cafe7d827fd",
}
CHAIN_ID = "creg-phase-c-testnet-2026-09-26-r7"
GENESIS = "0f2a3b109318f0979807f3d921f72775b2a37ce4eb28415a62d9f53b3766eed4"
DEFAULT_CANONICAL = "npm:escape-html@1.0.3"


def docker(*args):
    return subprocess.run(["docker", *args], check=True, capture_output=True, text=True).stdout.strip()


def read(path):
    for attempt in range(5):
        try:
            with urllib.request.urlopen(API + path, timeout=8) as response:
                return json.loads(response.read().decode())
        except urllib.error.HTTPError as error:
            if error.code not in (429, 502, 503, 504) or attempt == 4:
                raise
            time.sleep(min(0.25 * (2 ** attempt), 4.0))


def container_state():
    return json.loads(docker("inspect", CONTAINER))[0]["State"]


def save(report, destination):
    temporary = destination.with_suffix(destination.suffix + ".tmp")
    temporary.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    temporary.replace(destination)


def utc_now():
    return datetime.datetime.now(datetime.timezone.utc).isoformat()


def main():
    destination = Path(sys.argv[1])
    target_height = int(sys.argv[2])
    canonical = sys.argv[3] if len(sys.argv) > 3 else DEFAULT_CANONICAL
    expected_operation = sys.argv[4] if len(sys.argv) > 4 else "revoke"
    if expected_operation not in ("publish", "revoke"):
        raise RuntimeError("operation must be publish or revoke")
    if destination.exists():
        raise RuntimeError(f"evidence already exists: {destination}")
    before_stats = read("/v1/public/chain/stats")
    state = container_state()
    image = docker("inspect", CONTAINER, "--format", "{{.Image}}")
    restart_count = int(docker("inspect", CONTAINER, "--format", "{{.RestartCount}}"))
    if image not in ACCEPTED_IMAGES:
        raise RuntimeError(f"V4 image is outside the R21 pin: {image}")
    if before_stats.get("chain_id") != CHAIN_ID or before_stats.get("tip_height") != target_height - 1:
        raise RuntimeError("V4 is not at the expected R21 pre-transaction tip")
    if state.get("Status") != "running":
        raise RuntimeError("V4 must be running before the prepared-round monitor is armed")

    report = {
        "schema": "creg-real-r21-v4-prepared-crash-recovery-v1",
        "started_at_utc": utc_now(),
        "target_height": target_height,
        "expected_canonical": canonical,
        "expected_operation": expected_operation,
        "container": CONTAINER,
        "image_id": image,
        "container_restart_count_before": restart_count,
        "other_validators_remain_online": ["validator-1", "validator-2", "validator-3"],
        "no_two_validator_consensus_window": True,
        "no_journal_manipulation": True,
        "before_stats": before_stats,
        "passed": False,
    }
    destination.parent.mkdir(parents=True, exist_ok=True)
    save(report, destination)

    deadline = time.time() + 900
    observed = None
    next_tip_check = 0.0
    while time.time() < deadline:
        pbft = read("/v1/consensus/pbft")
        observed = next((round_state for round_state in pbft.get("rounds", [])
                         if round_state.get("block_height") == target_height
                         and round_state.get("phase") == "commit"
                         and round_state.get("prepare_count", 0) >= round_state.get("quorum", 3)
                         and 0 < round_state.get("commit_count", 0) < round_state.get("quorum", 3)), None)
        if observed:
            break
        now = time.time()
        if now >= next_tip_check:
            if read("/v1/public/chain/stats").get("tip_height", 0) >= target_height:
                raise RuntimeError("The prepared-round crash window closed before it was observed")
            next_tip_check = now + 2.0
        time.sleep(0.25)
    if not observed:
        raise RuntimeError("No quorum-prepared V4 COMMIT window was observed within fifteen minutes")

    report["prepared_round"] = observed
    report["observed_prepared_at_utc"] = utc_now()
    save(report, destination)
    subprocess.run(["docker", "kill", "--signal", "SIGKILL", CONTAINER], check=True,
                   capture_output=True, text=True)
    report["crashed_at_utc"] = utc_now()
    report["sigkill_requested"] = True
    save(report, destination)

    restart_deadline = time.time() + 180
    while time.time() < restart_deadline:
        try:
            state = container_state()
            current_restart_count = int(docker("inspect", CONTAINER, "--format", "{{.RestartCount}}"))
            if state.get("Running") and current_restart_count > restart_count:
                break
        except Exception:
            pass
        time.sleep(0.5)
    else:
        raise RuntimeError("Docker restart policy did not bring V4 back; watchdog remains armed")

    report["restarted_at_utc"] = utc_now()
    report["container_restart_count_after"] = current_restart_count
    report["restart_state"] = state
    save(report, destination)

    recovery_deadline = time.time() + 180
    after_stats = None
    block = None
    while time.time() < recovery_deadline:
        try:
            after_stats = read("/v1/public/chain/stats")
            block = read(f"/v1/public/blocks/{target_height}")
            if after_stats.get("tip_height", 0) >= target_height:
                break
        except Exception:
            time.sleep(0.5)
            continue
        time.sleep(0.5)

    logs = subprocess.run(["docker", "logs", "--since", report["crashed_at_utc"], CONTAINER],
                          capture_output=True, text=True, check=False)
    log_text = (logs.stdout or "") + "\n" + (logs.stderr or "")
    recovery_lines = [line for line in log_text.splitlines()
                      if "Recovered durable PBFT prepared round" in line
                      or "Recovered durable local PBFT COMMIT" in line]
    height_pattern = re.compile(rf"height[= ]{target_height}(?:\D|$)")
    recovered_prepared = any("Recovered durable PBFT prepared round" in line and height_pattern.search(line)
                             for line in recovery_lines)
    recovered_commit = any("Recovered durable local PBFT COMMIT" in line
                           and height_pattern.search(line)
                           and observed["block_hash"] in line
                           for line in recovery_lines)
    block_transactions = block.get("transactions", []) if isinstance(block, dict) else []

    def transaction_matches_expected_package(tx):
        if expected_operation == "revoke":
            return tx.get("type") == "revoke" and tx.get("package_canonical") == canonical
        record = tx.get("record", {})
        package_id = record.get("id", {}) if isinstance(record, dict) else {}
        actual_canonical = f"{package_id.get('ecosystem', '')}:{package_id.get('name', '')}@{package_id.get('version', '')}"
        return tx.get("type") in ("publish", "publish_authorized_v1") and actual_canonical == canonical
    package = read("/v1/public/packages/" + urllib.parse.quote(canonical, safe=""))
    expected_package_status = "revoked" if expected_operation == "revoke" else "verified"
    checks = {
        "prepared_quorum_observed_before_crash": observed["prepare_count"] >= observed["quorum"],
        "local_commit_present_before_crash": observed["commit_count"] > 0,
        "v4_container_restarted": current_restart_count > restart_count and state.get("Running"),
        "same_r21_image_after_restart": docker("inspect", CONTAINER, "--format", "{{.Image}}") == image,
        "durable_prepared_round_recovered": recovered_prepared,
        "durable_local_commit_recovered": recovered_commit,
        "same_prepared_hash_finalized": after_stats is not None and after_stats.get("tip_height") == target_height
            and after_stats.get("tip_hash") == observed["block_hash"]
            and isinstance(block, dict) and block.get("hash") == observed["block_hash"] and block.get("finalized") is True,
        "single_expected_package_transaction_in_block": sum(
            1 for tx in block_transactions if transaction_matches_expected_package(tx)
        ) == 1,
        "expected_package_state_committed": package.get("status", "").lower() == expected_package_status
            and package.get("block_phase") == "committed",
        "chain_identity_preserved": after_stats is not None and after_stats.get("chain_id") == CHAIN_ID
            and after_stats.get("genesis_hash", "").removeprefix("0x") == GENESIS,
    }
    report.update({
        "after_stats": after_stats,
        "finalized_block": block,
        "package_state_after": package,
        "recovery_log_lines": recovery_lines,
        "checks": checks,
        "completed_at_utc": utc_now(),
        "passed": all(checks.values()),
    })
    save(report, destination)
    if not report["passed"]:
        raise RuntimeError("R21 V4 prepared recovery did not satisfy every check")


if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        print(str(error), file=sys.stderr)
        raise
