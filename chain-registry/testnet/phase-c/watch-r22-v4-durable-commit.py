#!/usr/bin/env python3
"""Capture V4's armed R22 durable-COMMIT boundary and restart recovery."""

import datetime as dt
import json
import os
import pathlib
import re
import selectors
import subprocess
import sys
import time
import urllib.parse
import urllib.request

CONTAINER = "creg-phase-c-r7-v4"
IMAGE_ID = "sha256:8731cc85c73e6267e2e00014e8df50aef3f4d60ce6cfcaddb8fd0a3584eb2ad9"
CHAIN_ID = "creg-phase-c-testnet-2026-09-26-r7"
GENESIS = "1852b9a86dd3b53e7b2de4a774723a4fe48e862f9c61d14e5917b42b21055234"
CANONICAL = "npm:kleur@4.1.5"
HEIGHT = 14
PROFILE = "f397fac29df29438f0ee15580547def5fe66a03af60153d22993f4a7af9fce8c"
REPORT = pathlib.Path(
    "/var/lib/creg/phase-c-r7/w3-v4-durable-commit-capture.json"
)
FIELD_RE = re.compile(r'(?:^|\s)([a-z_]+)[=:]\s*(?:"([^"]*)"|([^\s,]+))', re.I)
BLOCK_HASH_RE = re.compile(r'block_hash[\s=:"\']+([0-9a-f]{64})', re.I)


def utc_now():
    return dt.datetime.now(dt.timezone.utc).isoformat()


def run(args, *, check=True):
    return subprocess.run(args, check=check, capture_output=True, text=True)


def docker_inspect():
    return json.loads(run(["docker", "inspect", CONTAINER]).stdout)[0]


def parse_fields(line):
    block_hash = BLOCK_HASH_RE.search(line)
    fields = {}
    for match in FIELD_RE.finditer(line):
        fields[match.group(1).lower()] = match.group(2) if match.group(2) is not None else match.group(3)
    try:
        height = int(fields["height"].strip('"'))
    except (KeyError, ValueError):
        height = None
    return {"fields": fields, "block_hash": block_hash.group(1).lower() if block_hash else None, "height": height}


def save(report):
    temporary = REPORT.with_suffix(REPORT.suffix + ".tmp")
    temporary.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    os.chmod(temporary, 0o600)
    os.replace(temporary, REPORT)
    os.chmod(REPORT, 0o600)


def api_json(path):
    with urllib.request.urlopen("http://127.0.0.1:8080" + path, timeout=8) as response:
        return json.loads(response.read().decode("utf-8"))


def main():
    if os.geteuid() != 0:
        raise SystemExit("Run this monitor as root on the V4 host.")

    info = docker_inspect()
    env = {}
    for entry in info.get("Config", {}).get("Env", []):
        if "=" in entry:
            key, value = entry.split("=", 1)
            env[key] = value

    expected_env = {
        "CREG_PHASE_C_COMMIT_CAPTURE_NODE_ID": "validator-4",
        "CREG_PHASE_C_COMMIT_CAPTURE_CHAIN_ID": CHAIN_ID,
        "CREG_PHASE_C_COMMIT_CAPTURE_GENESIS_HASH": GENESIS,
        "CREG_PHASE_C_COMMIT_CAPTURE_HEIGHT": str(HEIGHT),
        "CREG_PHASE_C_COMMIT_CAPTURE_PACKAGE_CANONICAL": CANONICAL,
        "CREG_PHASE_C_COMMIT_CAPTURE_PAUSE_MS": "30000",
        "CREG_SCANNER_PROFILE_DIGEST": PROFILE,
    }
    if info.get("Image") != IMAGE_ID or info.get("State", {}).get("Running") is not True:
        raise SystemExit("V4 is not running the reviewed R22 image.")
    if any(env.get(key) != value for key, value in expected_env.items()):
        raise SystemExit("V4 does not have the exact reviewed durable-COMMIT arm.")
    if REPORT.exists():
        raise SystemExit(f"Evidence file already exists; refusing to overwrite {REPORT}.")

    report = {
        "schema": "creg-r22-v4-durable-commit-crash-capture-v1",
        "armed_at_utc": utc_now(),
        "node_id": "validator-4",
        "chain_id": CHAIN_ID,
        "signed_spec_genesis_identity": GENESIS,
        "height": HEIGHT,
        "canonical": CANONICAL,
        "container_id": info.get("Id"),
        "image_id": info.get("Image"),
        "initial_restart_count": info.get("RestartCount"),
        "durable_marker": None,
        "pause_marker": None,
        "kill": None,
        "recovery_marker": None,
        "recovered_tip": None,
        "result": "waiting_for_exact_durable_commit_and_pause_markers",
    }
    save(report)

    logs = subprocess.Popen(
        ["docker", "logs", "--follow", "--tail", "0", "--timestamps", CONTAINER],
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        bufsize=1,
    )
    selector = selectors.DefaultSelector()
    selector.register(logs.stdout, selectors.EVENT_READ)
    deadline = time.monotonic() + 900
    durable_by_hash = {}

    try:
        while time.monotonic() < deadline and logs.poll() is None:
            if not selector.select(timeout=1):
                continue
            raw_line = logs.stdout.readline()
            if not raw_line:
                break
            line = raw_line.rstrip("\r\n")
            fields = parse_fields(line)
            values = fields["fields"]
            common_match = (
                fields["height"] == HEIGHT
                and values.get("chain_id") == CHAIN_ID
                and values.get("genesis_hash", "").lower().removeprefix("0x") == GENESIS
            )

            if "pbft_local_commit_durable" in line and common_match:
                if (
                    values.get("node_id") == "validator-4"
                    and fields["block_hash"]
                    and "Local PBFT COMMIT durably persisted before consensus application or gossip" in line
                ):
                    durable_by_hash[fields["block_hash"]] = line
                    report["durable_marker"] = {
                        "observed_at_utc": utc_now(),
                        "block_hash": fields["block_hash"],
                        "line": line,
                    }
                    report["result"] = "durable_commit_seen_waiting_for_pause_marker"
                    save(report)

            if "phase_c_durable_commit_capture_paused" in line and common_match:
                block_hash = fields["block_hash"]
                if (
                    values.get("node_id") == "validator-4"
                    and values.get("package_canonical") == CANONICAL
                    and block_hash
                    and block_hash in durable_by_hash
                    and "Durable local COMMIT is held before in-memory application and peer broadcast" in line
                ):
                    report["pause_marker"] = {
                        "observed_at_utc": utc_now(),
                        "block_hash": block_hash,
                        "line": line,
                    }
                    report["durable_marker"]["matching_pause_hash"] = block_hash
                    report["result"] = "pause_seen_sigkill_in_progress"
                    save(report)

                    kill_started = utc_now()
                    killed = run(
                        ["docker", "kill", "--signal", "SIGKILL", CONTAINER],
                        check=False,
                    )
                    report["kill"] = {
                        "started_at_utc": kill_started,
                        "completed_at_utc": utc_now(),
                        "exit_code": killed.returncode,
                        "container_id": info.get("Id"),
                        "signal": "SIGKILL",
                        "stdout": killed.stdout.strip(),
                        "stderr": killed.stderr.strip(),
                    }
                    if killed.returncode != 0:
                        report["result"] = "sigkill_failed"
                        save(report)
                        raise SystemExit("SIGKILL failed; V4 remains armed. Inspect the host before proceeding.")

                    logs.terminate()
                    try:
                        logs.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        logs.kill()
                        logs.wait(timeout=5)

                    restart_deadline = time.monotonic() + 150
                    recovered_line = None
                    recovered_info = None
                    while time.monotonic() < restart_deadline:
                        try:
                            recovered_info = docker_inspect()
                            if (
                                recovered_info.get("Id") != info.get("Id")
                                or recovered_info.get("Image") != IMAGE_ID
                                or recovered_info.get("RestartCount", 0) <= info.get("RestartCount", 0)
                                or recovered_info.get("State", {}).get("Running") is not True
                            ):
                                time.sleep(0.25)
                                continue
                            recent = run(["docker", "logs", "--tail", "1000", CONTAINER], check=False).stdout
                            for candidate in reversed(recent.splitlines()):
                                recovered_fields = parse_fields(candidate)
                                if (
                                    "pbft_local_commit_recovered" in candidate
                                    and recovered_fields["height"] == HEIGHT
                                    and recovered_fields["block_hash"] == block_hash
                                    and recovered_fields["fields"].get("validator_id", recovered_fields["fields"].get("node_id")) == "validator-4"
                                    and recovered_fields["fields"].get("chain_id") == CHAIN_ID
                                    and recovered_fields["fields"].get("genesis_hash", "").lower().removeprefix("0x") == GENESIS
                                    and "Recovered durable local PBFT COMMIT before P2P and sync services start" in candidate
                                ):
                                    recovered_line = candidate
                                    break
                            if recovered_line:
                                break
                        except Exception:
                            pass
                        time.sleep(0.25)

                    if not recovered_line:
                        report["result"] = "recovery_marker_not_observed"
                        save(report)
                        raise SystemExit("V4 restarted, but exact durable COMMIT replay was not observed.")

                    report["recovery_marker"] = {
                        "observed_at_utc": utc_now(),
                        "block_hash": block_hash,
                        "line": recovered_line,
                    }
                    report["restart_count_after_recovery"] = recovered_info.get("RestartCount")
                    report["container_running_after_recovery"] = recovered_info.get("State", {}).get("Running") is True
                    report["result"] = "replay_seen_waiting_for_local_canonical_tip"
                    save(report)

                    canonical_path = "/v1/public/packages/" + urllib.parse.quote(CANONICAL, safe="")
                    tip_deadline = time.monotonic() + 180
                    while time.monotonic() < tip_deadline:
                        try:
                            stats = api_json("/v1/public/chain/stats")
                            health = api_json("/v1/public/health")
                            pbft = api_json("/v1/consensus/pbft")
                            package = api_json(canonical_path)
                            with urllib.request.urlopen("http://127.0.0.1:8080/metrics", timeout=8) as response:
                                metrics = response.read().decode("utf-8")
                            if stats.get("tip_height") == HEIGHT and stats.get("tip_hash", "").lower().removeprefix("0x") == block_hash:
                                checks = {
                                    "same_reviewed_r22_container": recovered_info.get("Id") == info.get("Id") and recovered_info.get("Image") == IMAGE_ID,
                                    "restart_count_incremented": recovered_info.get("RestartCount", 0) > info.get("RestartCount", 0),
                                    "health_ok_for_v4": health.get("status") == "ok" and health.get("node_id") == "validator-4" and health.get("is_validator") is True,
                                    "same_r7_chain_and_signed_spec": stats.get("chain_id") == CHAIN_ID and stats.get("spec_signature_verified") is True,
                                    "local_h14_hash_matches_durable_commit": stats.get("finalized_height") == HEIGHT and stats.get("tip_hash", "").lower().removeprefix("0x") == block_hash,
                                    "scanner_profile_ready": f'profile_digest="{PROFILE}"' in metrics and "creg_scanner_osv_lookups 5" in metrics,
                                    "pbft_clear": pbft.get("active_round_count") == 0 and not pbft.get("rounds"),
                                    "package_revoked_in_committed_block": (package.get("status", "").lower() == "revoked" and package.get("block_phase") == "committed"),
                                }
                                report["recovered_tip"] = {
                                    "captured_at_utc": utc_now(),
                                    "stats": stats,
                                    "health": health,
                                    "pbft": pbft,
                                    "package_status": package.get("status"),
                                    "package_block_phase": package.get("block_phase"),
                                    "checks": checks,
                                }
                                report["result"] = "passed_local_durable_commit_replay_and_h14_application" if all(checks.values()) else "h14_applied_but_acceptance_checks_failed"
                                save(report)
                                print(json.dumps(report, indent=2), flush=True)
                                return 0 if all(checks.values()) else 1
                        except Exception:
                            pass
                        time.sleep(0.5)

                    report["result"] = "replay_seen_but_h14_not_applied_locally"
                    save(report)
                    raise SystemExit("Exact COMMIT replay was observed, but V4 did not apply its h14 block in time.")

            if time.monotonic() >= deadline:
                break
    finally:
        if logs.poll() is None:
            logs.terminate()
            try:
                logs.wait(timeout=5)
            except subprocess.TimeoutExpired:
                logs.kill()
                logs.wait(timeout=5)

    report["result"] = "timed_out_without_exact_commit_pause_boundary"
    report["finished_at_utc"] = utc_now()
    save(report)
    raise SystemExit("Timed out without the exact V4 h14 durable-COMMIT pause marker; V4 was not killed.")


if __name__ == "__main__":
    sys.exit(main())
