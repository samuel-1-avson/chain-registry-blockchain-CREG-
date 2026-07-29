#!/usr/bin/env python3
"""Acquire production trust candidates without activating either adapter.

Run this only in a clean Linux environment with the exact locked command shown
in the authority-adapter README. Sigstore material is downloaded through the
sigstore-python TUF client. npm registry keys are retrieved from npm's official
HTTPS endpoint, whose response does not provide a TUF-equivalent signature or
the missing Trusted Publishing authentication-mode assertion.
"""

from __future__ import annotations

import argparse
import base64
import hashlib
import json
import os
import tempfile
import urllib.request
from datetime import datetime, timezone
from pathlib import Path

from cryptography.hazmat.primitives.serialization import load_der_public_key
from cryptography.hazmat.primitives.asymmetric.ec import (
    SECP256R1,
    EllipticCurvePublicKey,
)
from sigstore.models import DEFAULT_TUF_URL, TrustUpdater, TrustedRoot


NPM_KEYS_URL = "https://registry.npmjs.org/-/npm/v1/keys"
SIGSTORE_TARGET = "trusted_root.json"
MAX_TRUST_ROOT_BYTES = 2 * 1024 * 1024
MAX_NPM_KEYS_BYTES = 256 * 1024


def sha256_hex(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def strict_json(data: bytes, label: str) -> object:
    def reject_constant(value: str) -> object:
        raise ValueError(f"{label} contains non-finite JSON number {value}")

    def reject_duplicate(pairs: list[tuple[str, object]]) -> dict[str, object]:
        result: dict[str, object] = {}
        for key, value in pairs:
            if key in result:
                raise ValueError(f"{label} contains duplicate member {key!r}")
            result[key] = value
        return result

    return json.loads(
        data,
        parse_constant=reject_constant,
        object_pairs_hook=reject_duplicate,
    )


def acquire_sigstore_root() -> bytes:
    updater = TrustUpdater(DEFAULT_TUF_URL)
    path = Path(updater.get_trusted_root_path())
    data = path.read_bytes()
    if not data or len(data) > MAX_TRUST_ROOT_BYTES:
        raise ValueError("TUF-verified Sigstore TrustedRoot has invalid size")
    parsed = strict_json(data, "Sigstore TrustedRoot")
    if not isinstance(parsed, dict):
        raise ValueError("Sigstore TrustedRoot is not a JSON object")
    # Parse the exact downloaded target through the same model class used by
    # sigstore-python. This supplements, but does not replace, TUF verification.
    TrustedRoot.from_file(str(path))
    return data


def acquire_npm_keys(now: datetime) -> tuple[bytes, list[str]]:
    request = urllib.request.Request(
        NPM_KEYS_URL,
        headers={
            "Accept": "application/json",
            "User-Agent": "creg-trust-candidate-acquisition/1",
        },
        method="GET",
    )
    with urllib.request.urlopen(request, timeout=30) as response:
        if response.geturl() != NPM_KEYS_URL:
            raise ValueError("npm registry-key request was redirected")
        data = response.read(MAX_NPM_KEYS_BYTES + 1)
    if not data or len(data) > MAX_NPM_KEYS_BYTES:
        raise ValueError("npm registry-key response has invalid size")

    parsed = strict_json(data, "npm registry keys")
    if not isinstance(parsed, dict) or set(parsed) != {"keys"}:
        raise ValueError("npm registry-key response has an unexpected shape")
    keys = parsed["keys"]
    if not isinstance(keys, list) or not 1 <= len(keys) <= 64:
        raise ValueError("npm registry-key response is empty or excessive")

    active: list[str] = []
    seen: set[str] = set()
    expected_fields = {"expires", "key", "keyid", "keytype", "scheme"}
    for index, item in enumerate(keys):
        if not isinstance(item, dict) or set(item) != expected_fields:
            raise ValueError(f"npm registry key {index} has an unexpected shape")
        key_id = item["keyid"]
        if not isinstance(key_id, str) or not key_id.startswith("SHA256:"):
            raise ValueError(f"npm registry key {index} has an invalid key id")
        if key_id in seen:
            raise ValueError("npm registry-key response contains a duplicate key id")
        seen.add(key_id)
        if (
            item["keytype"] != "ecdsa-sha2-nistp256"
            or item["scheme"] != "ecdsa-sha2-nistp256"
        ):
            raise ValueError(f"npm registry key {index} has an unsupported algorithm")
        try:
            der = base64.b64decode(item["key"], validate=True)
            public_key = load_der_public_key(der)
        except Exception as error:
            raise ValueError(f"npm registry key {index} is not valid DER SPKI") from error
        if not isinstance(public_key, EllipticCurvePublicKey) or not isinstance(
            public_key.curve, SECP256R1
        ):
            raise ValueError(f"npm registry key {index} is not ECDSA P-256")

        expires = item["expires"]
        if expires is None:
            active.append(key_id)
        elif isinstance(expires, str):
            expiry = datetime.fromisoformat(expires.replace("Z", "+00:00"))
            if expiry > now:
                active.append(key_id)
        else:
            raise ValueError(f"npm registry key {index} has an invalid expiry")

    if not active:
        raise ValueError("npm registry-key response contains no currently active key")
    return data, sorted(active)


def atomic_write(path: Path, data: bytes) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    descriptor, temporary_name = tempfile.mkstemp(
        prefix=f".{path.name}.", suffix=".tmp", dir=path.parent
    )
    temporary = Path(temporary_name)
    try:
        with os.fdopen(descriptor, "wb") as handle:
            handle.write(data)
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(temporary, path)
    finally:
        temporary.unlink(missing_ok=True)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--output",
        type=Path,
        required=True,
        help="empty or existing output directory for unapproved candidate files",
    )
    arguments = parser.parse_args()
    output = arguments.output.resolve()

    now = datetime.now(timezone.utc)
    root = acquire_sigstore_root()
    npm_keys, active_key_ids = acquire_npm_keys(now)
    root_name = "sigstore-production-trusted-root.json"
    npm_name = "npm-registry-keys.json"

    manifest = {
        "schema_version": 1,
        "status": "candidate_unapproved",
        "retrieved_at": now.isoformat(timespec="seconds").replace("+00:00", "Z"),
        "sigstore": {
            "source": f"{DEFAULT_TUF_URL}/targets/{SIGSTORE_TARGET}",
            "verification": "sigstore-python-4.5.0-tuf",
            "file": root_name,
            "bytes": len(root),
            "sha256": sha256_hex(root),
        },
        "npm_registry_keys": {
            "source": NPM_KEYS_URL,
            "verification": "official-https-endpoint-only",
            "file": npm_name,
            "bytes": len(npm_keys),
            "sha256": sha256_hex(npm_keys),
            "active_key_ids_at_retrieval": active_key_ids,
        },
        "activation_blockers": [
            "independent trust-root and npm-key review has not been recorded",
            "namespace-specific canonical publisher policies have not been approved",
            "npm exposes no signed signal proving Trusted Publishing authentication mode",
            "no signed CR-07 activation manifest or operator rehearsal exists",
        ],
    }
    manifest_bytes = (
        json.dumps(manifest, ensure_ascii=False, separators=(",", ":"), sort_keys=True)
        + "\n"
    ).encode("utf-8")

    atomic_write(output / root_name, root)
    atomic_write(output / npm_name, npm_keys)
    atomic_write(output / "candidate-manifest.json", manifest_bytes)
    print(sha256_hex(manifest_bytes))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
