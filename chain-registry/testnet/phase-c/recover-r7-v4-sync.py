#!/usr/bin/env python3
"""Restart only the existing R7 V4 container after a recent fleet preflight."""

import argparse
import datetime as dt
import hashlib
import json
from pathlib import Path
import re
import subprocess
import time
import urllib.request


def utc(value):
    return dt.datetime.fromisoformat(value.replace("Z", "+00:00"))


def docker(*args):
    return subprocess.check_output(["docker", *args], text=True).strip()


def get(path):
    with urllib.request.urlopen("http://127.0.0.1:8080" + path, timeout=15) as reply:
        return json.load(reply)


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def metric(text, name):
    match = re.search(r"^" + re.escape(name) + r"\s+([0-9.]+)$", text, re.M)
    return float(match.group(1)) if match else None


def current():
    inspected = json.loads(docker("inspect", "creg-phase-c-r7-v4"))[0]
    with urllib.request.urlopen("http://127.0.0.1:8080/metrics", timeout=15) as reply:
        metrics = reply.read().decode()
    root = Path("/var/lib/creg/phase-c-r7")
    environment = inspected["Config"].get("Env") or []
    env_digest = hashlib.sha256(json.dumps(sorted(environment)).encode()).hexdigest()
    return {
        "captured_at_utc": dt.datetime.now(dt.timezone.utc).isoformat(),
        "container_id": inspected["Id"],
        "image_id": inspected["Image"],
        "running": inspected["State"]["Running"],
        "mounts": sorted((m["Source"], m["Destination"], m["RW"]) for m in inspected["Mounts"]),
        "identity_sha256": digest(root / "data/p2p/identity.key"),
        "environment_sha256": digest(root / "node.env"),
        "container_environment_sha256": env_digest,
        "journal_present": (root / "data/pbft-safety.rocksdb").is_dir(),
        "small_cluster_quorum_disabled": "CREG_PBFT_ALLOW_SMALL_CLUSTER_QUORUM=false" in environment,
        "health": get("/v1/public/health"),
        "stats": get("/v1/public/chain/stats"),
        "pbft": get("/v1/consensus/pbft"),
        "queues": {name: metric(metrics, name) for name in (
            "creg_pending_block_txs", "creg_durable_finalized_work_depth",
            "creg_forced_inclusion_pending", "creg_durable_finalized_work_dead_letter_depth")},
    }


def sync_current(health, now):
    sync = health.get("validator_set_sync") or {}
    return (sync.get("enabled") is True and sync.get("mode") == "chain-authoritative"
            and sync.get("state") == "synced" and sync.get("last_error") is None
            and sync.get("last_poll_at") is not None
            and 0 <= (now - utc(sync["last_poll_at"])).total_seconds() <= 120
            and sync.get("cursor_block", -1) >= sync.get("last_finalized_source_block", 0))


def canonical(stats, reference):
    return (stats.get("chain_id") == "creg-phase-c-testnet-2026-09-26-r7"
            and stats.get("genesis_hash") == "0f2a3b109318f0979807f3d921f72775b2a37ce4eb28415a62d9f53b3766eed4"
            and stats.get("spec_signature_verified") is True
            and stats.get("tip_hash") == reference.get("tip_hash")
            and stats.get("tip_height") == stats.get("finalized_height") == reference.get("tip_height")
            and stats.get("active_validators") == stats.get("validator_count") == 4)


def clear(record):
    for round_state in record["pbft"].get("rounds", []):
        phase = round_state.get("phase")
        if phase != "finalised" and (phase != "failed" or round_state.get("prepare_count", 0) >= round_state.get("quorum", 3)):
            return False
    return all(record["queues"][name] == 0 for name in (
        "creg_pending_block_txs", "creg_durable_finalized_work_depth", "creg_forced_inclusion_pending"))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--fleet-preflight", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--restart", action="store_true", help="Perform one restart after all checks pass")
    args = parser.parse_args()
    if args.output.exists():
        raise SystemExit("Output already exists; preserve prior evidence")
    now = dt.datetime.now(dt.timezone.utc)
    fleet = json.loads(args.fleet_preflight.read_text())
    if not fleet.get("all_eight_shared_finalized_tip") or not fleet.get("all_eight_recovery_clear") or not fleet.get("all_eight_pbft_clear"):
        raise SystemExit("Fleet must have a shared finalized tip and no unresolved recovery/PBFT work")
    nodes = {node["slot"]: node for node in fleet["nodes"]}
    if set(nodes) != {"v1", "v2", "v3", "v4", "o1", "o2", "o3", "o4"}:
        raise SystemExit("Complete eight-node preflight required")
    reference = nodes["v4"]["stats"]
    for slot in ("v1", "v2", "v3"):
        node = nodes[slot]
        age = (now - utc(node["captured_at_utc"])).total_seconds()
        if not (0 <= age <= 600 and node["health"].get("is_validator") is True
                and node["container_state"].get("Running") is True
                and sync_current(node["health"], utc(node["captured_at_utc"]))
                and canonical(node["stats"], reference)):
            raise SystemExit("Other three validators require recent healthy, current-sync captures")
    before = current()
    if not (before["running"] and before["journal_present"] and before["small_cluster_quorum_disabled"]
            and before["health"].get("node_id") == "validator-4"
            and before["health"].get("is_validator") is True
            and before["image_id"] == nodes["v4"]["image_id"]
            and canonical(before["stats"], reference) and clear(before)):
        raise SystemExit("V4 preflight failed; no restart performed")
    root = Path("/var/lib/creg/phase-c-r7/data")
    if (str(root), "/data", True) not in before["mounts"]:
        raise SystemExit("Expected persistent data mount missing")
    report = {"schema": "creg-r7-v4-sync-recovery-v1", "before": before, "restart_performed": False, "passed": False}
    if not args.restart:
        report["preflight_passed"] = True
        args.output.write_text(json.dumps(report, indent=2) + "\n")
        print(json.dumps({"preflight_passed": True, "restart_performed": False}))
        return
    report["restart_performed"] = True
    # Same container, image, environment and persisted signing journal.
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    docker("restart", "--time", "30", "creg-phase-c-r7-v4")
    deadline = time.monotonic() + 300
    while time.monotonic() < deadline:
        try:
            after = current()
            unchanged = all(after[key] == before[key] for key in (
                "container_id", "image_id", "mounts", "identity_sha256", "environment_sha256", "container_environment_sha256"))
            success = (unchanged and after["journal_present"] and after["running"]
                       and canonical(after["stats"], reference) and clear(after)
                       and after["queues"] == before["queues"]
                       and sync_current(after["health"], dt.datetime.now(dt.timezone.utc)))
            report["after"] = after
            report["passed"] = success
            args.output.write_text(json.dumps(report, indent=2) + "\n")
            if success:
                print(json.dumps({"passed": True, "restart_performed": True,
                                  "tip_height": after["stats"]["tip_height"],
                                  "validator_set_sync": after["health"]["validator_set_sync"]}))
                return
        except (OSError, ValueError, subprocess.CalledProcessError):
            pass
        time.sleep(5)
    raise SystemExit("V4 restart completed but current-sync acceptance timed out; retained state preserved")


if __name__ == "__main__":
    main()
