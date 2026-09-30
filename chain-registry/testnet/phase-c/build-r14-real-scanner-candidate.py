#!/usr/bin/env python3
"""Build a new immutable scanner bundle pinned to a captured real OSV response."""

import datetime as dt
import hashlib
import json
import pathlib
import shutil
import sys


def sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def profile_digest(bundle: pathlib.Path, epoch: str):
    files = sorted(bundle.joinpath("rules").glob("*.yar"), key=lambda path: path.name)
    if not files:
        raise ValueError("YARA rule set is empty")
    hasher = hashlib.sha256()
    hasher.update(b"creg-yara-rule-bundle-v1\0")
    for path in files:
        for item in (path.name.encode(), path.read_bytes()):
            hasher.update(len(item).to_bytes(8, "big"))
            hasher.update(item)
    yara = hasher.hexdigest()
    threat = sha(bundle.joinpath("known_malicious_hashes.json").read_bytes())
    osv = sha(bundle.joinpath("osv_snapshot.json").read_bytes())
    profile = (
        "creg-scanner-profile-v1|version=creg-deterministic-scanner-v1.1.0"
        f"|yara={yara}|threat={threat}|osv_enabled=true|osv_epoch={epoch}"
        f"|osv={osv}|osv_block_critical=true|osv_live_fallback=false"
    )
    return sha(profile.encode()), yara, threat, osv


def main() -> int:
    if len(sys.argv) != 5:
        raise SystemExit(
            "Usage: build-r14-real-scanner-candidate.py PACKAGE_EVIDENCE_DIR "
            "BASELINE_BUNDLE OUTPUT_BUNDLE OSV_EPOCH"
        )
    package_dir, baseline_dir, output_dir = map(pathlib.Path, sys.argv[1:4])
    epoch = sys.argv[4]
    if not epoch or any(char.isspace() for char in epoch):
        raise ValueError("OSV epoch must be a non-empty token")
    if output_dir.exists():
        raise FileExistsError(f"Refusing to overwrite existing scanner bundle: {output_dir}")

    review = json.loads(package_dir.joinpath("review.json").read_text(encoding="utf-8"))
    query = json.loads(package_dir.joinpath("osv-query.json").read_text(encoding="utf-8"))
    response_bytes = package_dir.joinpath("osv-response.json").read_bytes()
    response = json.loads(response_bytes)
    canonical = review["canonical"]
    expected_query = {
        "package": {"ecosystem": "npm", "name": review["canonical"].split(":", 1)[1].rsplit("@", 1)[0]},
        "version": review["canonical"].rsplit("@", 1)[1],
    }
    if review.get("status") != "prepared_for_review_not_published":
        raise ValueError("Package evidence is not in the prepared, unpublished state")
    if review.get("fleet_changed") is not False or review.get("package_executed_or_published") is not False:
        raise ValueError("Package evidence already records a live action")
    if query != expected_query or review.get("osv_query") != expected_query:
        raise ValueError("Captured OSV query does not match the exact package/version")
    if sha(response_bytes) != review.get("osv_response_sha256"):
        raise ValueError("Captured OSV response hash does not match its review")
    vulnerabilities = response.get("vulns", [])
    if not isinstance(vulnerabilities, list):
        raise ValueError("OSV response has an invalid vulnerabilities field")

    baseline_manifest = json.loads(baseline_dir.joinpath("manifest.json").read_text(encoding="utf-8"))
    baseline_snapshot = json.loads(baseline_dir.joinpath("osv_snapshot.json").read_text(encoding="utf-8"))
    baseline_digest, baseline_yara, baseline_threat, baseline_osv = profile_digest(
        baseline_dir, baseline_snapshot["epoch"]
    )
    for key, expected in (
        ("scanner_profile_digest", baseline_digest),
        ("yara_bundle_sha256", baseline_yara),
        ("threat_database_sha256", baseline_threat),
        ("osv_snapshot_sha256", baseline_osv),
    ):
        if baseline_manifest.get(key) != expected:
            raise ValueError(f"Baseline scanner bundle integrity failure: {key}")
    if canonical in baseline_snapshot.get("entries", {}):
        raise ValueError("Exact package version is already represented in baseline snapshot")

    output_dir.mkdir(parents=True)
    shutil.copytree(baseline_dir.joinpath("rules"), output_dir.joinpath("rules"))
    shutil.copy2(baseline_dir.joinpath("known_malicious_hashes.json"), output_dir)
    snapshot = dict(baseline_snapshot)
    snapshot["epoch"] = epoch
    snapshot["built_at"] = dt.datetime.now(dt.timezone.utc).isoformat().replace("+00:00", "Z")
    snapshot["entries"] = dict(baseline_snapshot["entries"])
    snapshot["entries"][canonical] = vulnerabilities
    output_dir.joinpath("osv_snapshot.json").write_text(
        json.dumps(snapshot, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
    )

    digest, yara, threat, osv = profile_digest(output_dir, epoch)
    now = dt.datetime.now(dt.timezone.utc).isoformat().replace("+00:00", "Z")
    manifest = dict(baseline_manifest)
    manifest.update(
        generated_at_utc=now,
        threat_database_sha256=threat,
        osv_epoch=epoch,
        osv_snapshot_sha256=osv,
        yara_bundle_sha256=yara,
        scanner_profile_digest=digest,
        osv_coverage=list(baseline_manifest.get("osv_coverage", []))
        + [{"key": canonical, "vulnerability_count": len(vulnerabilities)}],
    )
    output_dir.joinpath("manifest.json").write_text(
        json.dumps(manifest, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
    )
    result = {
        "schema": "creg-real-scanner-r14-candidate-v1",
        "captured_at_utc": now,
        "status": "prepared_not_deployed",
        "canonical": canonical,
        "baseline_profile_digest": baseline_digest,
        "new_profile_digest": digest,
        "osv_epoch": epoch,
        "osv_response_sha256": sha(response_bytes),
        "osv_advisory_count": len(vulnerabilities),
        "osv_snapshot_sha256": osv,
        "threat_database_sha256": threat,
        "yara_bundle_sha256": yara,
        "coverage_count": len(snapshot["entries"]),
        "node_restarts_or_publication": False,
    }
    output_dir.joinpath("review.json").write_text(
        json.dumps(result, indent=2) + "\n", encoding="utf-8"
    )
    print(json.dumps(result, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
