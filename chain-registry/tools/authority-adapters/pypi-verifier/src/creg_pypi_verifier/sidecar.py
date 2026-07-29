"""Fail-closed CREG verifier for PyPI PEP 740 publish attestations.

The process accepts exactly one CREG authority-adapter v2 invocation on stdin.
It performs no network requests and writes stdout only after every binding,
archive, identity, and Sigstore verification succeeds.
"""

from __future__ import annotations

import gzip
import hashlib
import io
import json
import os
import re
import stat
import struct
import sys
import tarfile
import time
import unicodedata
import zipfile
from pathlib import Path, PurePosixPath
from typing import Any
from urllib.parse import urlsplit

import rfc8785
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey
from packaging.utils import canonicalize_name, parse_sdist_filename, parse_wheel_filename
from pydantic import TypeAdapter
from pypi_attestations import Distribution, Provenance, Publisher
from sigstore.models import TrustedRoot
from sigstore.verify import Verifier, policy as sigstore_policy
from sigstore_models.trustroot import v1 as trustroot_v1

PROTOCOL_VERSION = 2
AUTHORITY = "pypi_trusted_publisher"
AUDIENCE = "pypi"
PUBLISH_PREDICATE = "https://docs.pypi.org/attestations/publish/v1"
IN_TOTO_STATEMENT = "https://in-toto.io/Statement/v1"
DSSE_JSON_TYPE = "application/vnd.in-toto+json"
POLICY_FILENAME = "pypi-policy.json"
TRUST_ROOT_FILENAME = "pypi-trusted-root.json"
HARD_MAX_EVIDENCE_BYTES = 8 * 1024 * 1024
HARD_MAX_ARTIFACT_BYTES = 64 * 1024 * 1024
MAX_JCS_INVOCATION_OVERHEAD_BYTES = 1024 * 1024
# Rust transports exact binary inputs as lowercase hex. Reserve twice the two
# binary hard maxima plus a deliberately conservative MiB for the bounded v2
# envelope, URLs, identity strings, challenge, JSON syntax, and JCS escaping.
MAX_STDIN_BYTES = (
    2 * (HARD_MAX_EVIDENCE_BYTES + HARD_MAX_ARTIFACT_BYTES)
    + MAX_JCS_INVOCATION_OVERHEAD_BYTES
)
MAX_POLICY_BYTES = 1024 * 1024
MAX_TRUST_ROOT_BYTES = 4 * 1024 * 1024
MAX_CHALLENGE_BYTES = 64 * 1024
MAX_CHALLENGE_TTL_SECONDS = 15 * 60
MAX_CLOCK_SKEW_SECONDS = 60
MAX_SAFE_INTEGER = (1 << 53) - 1

_SHA256_RE = re.compile(r"^[0-9a-f]{64}$")
_ED25519_KEY_RE = re.compile(r"^[0-9a-f]{64}$")
_ED25519_SIGNATURE_RE = re.compile(r"^[0-9a-f]{128}$")
_EVM_ADDRESS_RE = re.compile(r"^0x[0-9a-f]{40}$")
_NAMESPACE_RE = re.compile(r"^pypi:([a-z0-9]+(?:-[a-z0-9]+)*)@$")

_INVOCATION_KEYS = {
    "schema_version",
    "adapter_id",
    "adapter_version",
    "policy_sha256",
    "trust_root_sha256",
    "request",
}
_REQUEST_KEYS = {
    "schema_version",
    "authority",
    "evidence_url",
    "evidence_sha256",
    "evidence_hex",
    "artifact_url",
    "artifact_digest_algorithm",
    "artifact_digest",
    "artifact_hex",
    "expected_issuer",
    "expected_subject",
    "expected_audience",
    "challenge",
}
_CHALLENGE_KEYS = {
    "schema_version",
    "purpose",
    "challenge_document",
    "chain_id",
    "genesis_hash",
    "namespace",
    "current_controller_ed25519_pubkey",
    "current_controller_evm_address",
    "next_controller_ed25519_pubkey",
    "next_controller_evm_address",
    "nonce",
    "issued_at",
    "expires_at",
    "next_controller_signature",
}
_POLICY_KEYS = {
    "schema_version",
    "authority",
    "adapter_id",
    "adapter_version",
    "sigstore_instance",
    "predicate_type",
    "publishers",
    "archive",
}
_ARCHIVE_POLICY_KEYS = {
    "max_entries",
    "max_entry_bytes",
    "max_uncompressed_bytes",
    "max_compression_ratio",
}
_PUBLISHER_POLICY_KEYS = {"namespace", "issuer", "subject", "publisher"}
_PROVENANCE_KEYS = {"version", "attestation_bundles"}
_BUNDLE_KEYS = {"publisher", "attestations"}
_ATTESTATION_KEYS = {"version", "verification_material", "envelope"}
_VERIFICATION_MATERIAL_KEYS = {"certificate", "transparency_entries"}
_ENVELOPE_KEYS = {"statement", "signature"}
_STATEMENT_KEYS = {"_type", "subject", "predicateType", "predicate"}
_SUBJECT_KEYS = {"name", "digest"}
_PUBLISHER_KEYS = {
    "GitHub": {"kind", "repository", "workflow", "environment"},
    "GitLab": {"kind", "repository", "workflow_filepath", "environment"},
    "Google": {"kind", "email"},
    "CircleCI": {
        "kind",
        "project_id",
        "pipeline_definition_id",
        "vcs_origin",
        "vcs_ref",
    },
}
_ZIP_EOCD = struct.Struct("<4s4H2LH")
_ZIP64_EOCD = struct.Struct("<4sQ2H2L4Q")
_ZIP64_LOCATOR = struct.Struct("<4sLQL")
_ZIP_CENTRAL_HEADER_BYTES = 46


class Rejected(ValueError):
    """A verification or configuration failure safe to report to stderr."""


def _reject(message: str) -> None:
    raise Rejected(message)


def _strict_object(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for key, value in pairs:
        if key in result:
            _reject("JSON contains a duplicate object member")
        result[key] = value
    return result


def _reject_nonfinite_json(_value: str) -> None:
    _reject("JSON contains a non-finite number")


def _parse_json(raw: bytes, label: str) -> Any:
    try:
        return json.loads(
            raw,
            object_pairs_hook=_strict_object,
            parse_constant=_reject_nonfinite_json,
        )
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise Rejected(f"{label} is not strict UTF-8 JSON") from error


def _require_object(value: Any, label: str, keys: set[str]) -> dict[str, Any]:
    if not isinstance(value, dict):
        _reject(f"{label} must be an object")
    if set(value) != keys:
        _reject(f"{label} has missing or unknown fields")
    return value


def _require_object_optional(
    value: Any,
    label: str,
    required: set[str],
    optional: set[str],
) -> dict[str, Any]:
    if not isinstance(value, dict):
        _reject(f"{label} must be an object")
    present = set(value)
    if not required <= present or not present <= required | optional:
        _reject(f"{label} has missing or unknown fields")
    return value


def _require_string(value: Any, label: str, maximum: int) -> str:
    if (
        not isinstance(value, str)
        or not value
        or len(value.encode("utf-8")) > maximum
        or value != value.strip()
        or any(character.isspace() and character not in {" "} for character in value)
        or any(unicodedata.category(character).startswith("C") for character in value)
    ):
        _reject(f"{label} is not canonical text")
    return value


def _require_int(value: Any, label: str, minimum: int, maximum: int) -> int:
    if type(value) is not int or value < minimum or value > maximum:
        _reject(f"{label} is outside its permitted integer range")
    return value


def _require_digest(value: Any, label: str) -> str:
    if not isinstance(value, str) or _SHA256_RE.fullmatch(value) is None:
        _reject(f"{label} must be a lowercase SHA-256 digest")
    return value


def _enforce_stdin_size(size: int) -> None:
    if size < 1 or size > MAX_STDIN_BYTES:
        _reject("stdin is empty or exceeds the protocol limit")


def _read_all_stdin() -> bytes:
    raw = sys.stdin.buffer.read(MAX_STDIN_BYTES + 1)
    _enforce_stdin_size(len(raw))
    return raw


def _read_regular_file(path: Path, maximum: int, label: str) -> bytes:
    try:
        before = path.stat(follow_symlinks=False)
    except OSError as error:
        raise Rejected(f"{label} is unavailable") from error
    if not stat.S_ISREG(before.st_mode) or path.is_symlink() or before.st_size > maximum:
        _reject(f"{label} must be a bounded, non-symlink regular file")
    try:
        with path.open("rb", buffering=0) as handle:
            raw = handle.read(maximum + 1)
            during = os.fstat(handle.fileno())
    except OSError as error:
        raise Rejected(f"{label} could not be read") from error
    try:
        after = path.stat(follow_symlinks=False)
    except OSError as error:
        raise Rejected(f"{label} changed while it was being read") from error
    identity_before = (before.st_dev, before.st_ino, before.st_size, before.st_mtime_ns)
    identity_during = (during.st_dev, during.st_ino, during.st_size, during.st_mtime_ns)
    identity_after = (after.st_dev, after.st_ino, after.st_size, after.st_mtime_ns)
    if (
        len(raw) > maximum
        or identity_before != identity_during
        or identity_before != identity_after
    ):
        _reject(f"{label} changed or exceeded its limit while it was being read")
    return raw


def _load_pinned_file(
    directory: Path,
    filename: str,
    expected_sha256: str,
    maximum: int,
    label: str,
) -> tuple[Path, bytes]:
    path = directory / filename
    raw = _read_regular_file(path, maximum, label)
    if hashlib.sha256(raw).hexdigest() != expected_sha256:
        _reject(f"{label} does not match the invocation pin")
    return path, raw


def _canonical_namespace(value: Any) -> str:
    namespace = _require_string(value, "namespace", 512)
    match = _NAMESPACE_RE.fullmatch(namespace)
    if match is None or canonicalize_name(match.group(1)) != match.group(1):
        _reject("namespace is not a canonical PyPI namespace")
    return namespace


def _validate_publisher_shape(value: Any, label: str) -> dict[str, Any]:
    if not isinstance(value, dict) or not isinstance(value.get("kind"), str):
        _reject(f"{label} is not a supported PyPI publisher identity")
    allowed = _PUBLISHER_KEYS.get(value["kind"])
    if allowed is None or "kind" not in value or not set(value) <= allowed:
        _reject(f"{label} has unknown publisher fields")
    required = {
        "GitHub": {"kind", "repository", "workflow"},
        "GitLab": {"kind", "repository", "workflow_filepath"},
        "Google": {"kind", "email"},
        "CircleCI": {"kind", "project_id", "pipeline_definition_id"},
    }[value["kind"]]
    if not required <= set(value):
        _reject(f"{label} is missing publisher fields")
    for key, item in value.items():
        if key == "kind":
            continue
        if item is not None:
            _require_string(item, f"{label}.{key}", 1024)
    return value


def _normal_publisher(value: Any, label: str) -> tuple[dict[str, Any], Any]:
    raw = _validate_publisher_shape(value, label)
    try:
        publisher = TypeAdapter(Publisher).validate_python(raw)
    except Exception as error:
        raise Rejected(f"{label} is not accepted by pypi-attestations") from error
    normalized = publisher.model_dump(exclude_none=True)
    return normalized, publisher


def _validate_policy(
    raw: bytes,
    invocation: dict[str, Any],
) -> dict[str, Any]:
    value = _require_object(_parse_json(raw, "policy"), "policy", _POLICY_KEYS)
    try:
        canonical = rfc8785.dumps(value)
    except Exception as error:
        raise Rejected("policy cannot be represented as RFC 8785 JSON") from error
    if canonical != raw:
        _reject("policy file is not canonical RFC 8785 JSON")
    if type(value["schema_version"]) is not int or value["schema_version"] != 1:
        _reject("policy schema version is unsupported")
    if value["authority"] != AUTHORITY:
        _reject("policy authority is not PyPI trusted publishing")
    if value["adapter_id"] != invocation["adapter_id"]:
        _reject("policy adapter id does not match the invocation")
    if value["adapter_version"] != invocation["adapter_version"]:
        _reject("policy adapter version does not match the invocation")
    if value["sigstore_instance"] != "production":
        _reject("policy does not require the Sigstore production instance")
    if value["predicate_type"] != PUBLISH_PREDICATE:
        _reject("policy does not require the PyPI publish predicate")

    archive = _require_object(value["archive"], "policy.archive", _ARCHIVE_POLICY_KEYS)
    _require_int(archive["max_entries"], "archive.max_entries", 1, 100_000)
    _require_int(
        archive["max_entry_bytes"],
        "archive.max_entry_bytes",
        MAX_CHALLENGE_BYTES,
        64 * 1024 * 1024,
    )
    _require_int(
        archive["max_uncompressed_bytes"],
        "archive.max_uncompressed_bytes",
        archive["max_entry_bytes"],
        512 * 1024 * 1024,
    )
    _require_int(
        archive["max_compression_ratio"],
        "archive.max_compression_ratio",
        1,
        1_000,
    )

    publishers = value["publishers"]
    if not isinstance(publishers, list) or not publishers or len(publishers) > 10_000:
        _reject("policy.publishers must be a bounded non-empty list")
    identities: set[bytes] = set()
    for index, entry_value in enumerate(publishers):
        entry = _require_object(
            entry_value,
            f"policy.publishers[{index}]",
            _PUBLISHER_POLICY_KEYS,
        )
        _canonical_namespace(entry["namespace"])
        _require_string(entry["issuer"], f"policy.publishers[{index}].issuer", 512)
        _require_string(entry["subject"], f"policy.publishers[{index}].subject", 1024)
        normalized, _ = _normal_publisher(
            entry["publisher"],
            f"policy.publishers[{index}].publisher",
        )
        identity = rfc8785.dumps(
            {
                "namespace": entry["namespace"],
                "issuer": entry["issuer"],
                "subject": entry["subject"],
                "publisher": normalized,
            }
        )
        if identity in identities:
            _reject("policy contains a duplicate publisher identity")
        identities.add(identity)
    return value


def _validate_challenge(challenge_value: Any, now: int) -> tuple[dict[str, Any], str]:
    challenge = _require_object(
        challenge_value,
        "request.challenge",
        _CHALLENGE_KEYS,
    )
    if type(challenge["schema_version"]) is not int or challenge["schema_version"] != 1:
        _reject("challenge schema version is unsupported")
    purpose = challenge["purpose"]
    if purpose not in {"claim", "recover"}:
        _reject("challenge purpose is unsupported")
    document = "creg-claim.json" if purpose == "claim" else "creg-recovery.json"
    if challenge["challenge_document"] != document:
        _reject("challenge document does not match its purpose")
    _require_string(challenge["chain_id"], "challenge.chain_id", 128)
    _require_digest(challenge["genesis_hash"], "challenge.genesis_hash")
    _canonical_namespace(challenge["namespace"])

    next_key = challenge["next_controller_ed25519_pubkey"]
    next_address = challenge["next_controller_evm_address"]
    signature = challenge["next_controller_signature"]
    if not isinstance(next_key, str) or _ED25519_KEY_RE.fullmatch(next_key) is None:
        _reject("challenge next-controller Ed25519 key is invalid")
    if not isinstance(next_address, str) or _EVM_ADDRESS_RE.fullmatch(next_address) is None:
        _reject("challenge next-controller EVM address is invalid")
    if not isinstance(signature, str) or _ED25519_SIGNATURE_RE.fullmatch(signature) is None:
        _reject("challenge next-controller signature is invalid")

    current_key = challenge["current_controller_ed25519_pubkey"]
    current_address = challenge["current_controller_evm_address"]
    if purpose == "claim":
        if current_key != "" or current_address != "":
            _reject("claim challenge unexpectedly names a current controller")
    elif (
        not isinstance(current_key, str)
        or _ED25519_KEY_RE.fullmatch(current_key) is None
        or not isinstance(current_address, str)
        or _EVM_ADDRESS_RE.fullmatch(current_address) is None
    ):
        _reject("recovery challenge current controller is invalid")

    nonce = _require_int(challenge["nonce"], "challenge.nonce", 1, MAX_SAFE_INTEGER)
    issued_at = _require_int(challenge["issued_at"], "challenge.issued_at", 0, MAX_SAFE_INTEGER)
    expires_at = _require_int(
        challenge["expires_at"],
        "challenge.expires_at",
        1,
        MAX_SAFE_INTEGER,
    )
    if (
        expires_at <= issued_at
        or expires_at - issued_at > MAX_CHALLENGE_TTL_SECONDS
        or now >= expires_at
        or issued_at > now + MAX_CLOCK_SKEW_SECONDS
    ):
        _reject("challenge is expired, premature, or too long-lived")

    intent = (
        "creg-namespace-authority-intent-v1\n"
        "service:creg-package-ownership\n"
        f"purpose:{purpose}\n"
        f"document:{document}\n"
        f"chain_id:{challenge['chain_id']}\n"
        f"genesis_hash:{challenge['genesis_hash']}\n"
        f"namespace:{challenge['namespace']}\n"
        f"current_ed25519_pubkey:{current_key}\n"
        f"current_evm_address:{current_address}\n"
        f"next_ed25519_pubkey:{next_key}\n"
        f"next_evm_address:{next_address}\n"
        f"nonce:{nonce}\n"
        f"issued_at:{issued_at}\n"
        f"expires_at:{expires_at}"
    ).encode("utf-8")
    try:
        Ed25519PublicKey.from_public_bytes(bytes.fromhex(next_key)).verify(
            bytes.fromhex(signature),
            intent,
        )
    except Exception as error:
        raise Rejected("challenge next-controller signature verification failed") from error
    return challenge, hashlib.sha256(intent).hexdigest()


def _decode_hex(value: Any, label: str, maximum: int) -> bytes:
    if (
        not isinstance(value, str)
        or not value
        or len(value) % 2
        or len(value) // 2 > maximum
        or value.lower() != value
        or any(character not in "0123456789abcdef" for character in value)
    ):
        _reject(f"{label} is not bounded lowercase hexadecimal")
    try:
        return bytes.fromhex(value)
    except ValueError as error:
        raise Rejected(f"{label} is not hexadecimal") from error


def _validate_invocation(raw: bytes, now: int) -> tuple[dict[str, Any], dict[str, Any], str]:
    invocation = _require_object(
        _parse_json(raw, "invocation"),
        "invocation",
        _INVOCATION_KEYS,
    )
    if invocation["schema_version"] != PROTOCOL_VERSION:
        _reject("invocation schema version is unsupported")
    _require_string(invocation["adapter_id"], "adapter_id", 128)
    _require_string(invocation["adapter_version"], "adapter_version", 64)
    _require_digest(invocation["policy_sha256"], "policy_sha256")
    _require_digest(invocation["trust_root_sha256"], "trust_root_sha256")

    request = _require_object(invocation["request"], "request", _REQUEST_KEYS)
    if request["schema_version"] != PROTOCOL_VERSION or request["authority"] != AUTHORITY:
        _reject("request is not a PyPI authority-adapter v2 request")
    if request["artifact_digest_algorithm"] != "sha256":
        _reject("PyPI artifact digest algorithm must be SHA-256")
    artifact_digest = _require_digest(request["artifact_digest"], "artifact_digest")
    evidence_digest = _require_digest(request["evidence_sha256"], "evidence_sha256")
    evidence = _decode_hex(
        request["evidence_hex"],
        "evidence_hex",
        HARD_MAX_EVIDENCE_BYTES,
    )
    artifact = _decode_hex(
        request["artifact_hex"],
        "artifact_hex",
        HARD_MAX_ARTIFACT_BYTES,
    )
    if hashlib.sha256(evidence).hexdigest() != evidence_digest:
        _reject("exact evidence bytes do not match evidence_sha256")
    if hashlib.sha256(artifact).hexdigest() != artifact_digest:
        _reject("exact artifact bytes do not match artifact_digest")
    _require_string(request["expected_issuer"], "expected_issuer", 512)
    _require_string(request["expected_subject"], "expected_subject", 1024)
    if request["expected_audience"] != AUDIENCE:
        _reject("expected audience is not PyPI")
    challenge, challenge_digest = _validate_challenge(request["challenge"], now)
    request["_evidence_bytes"] = evidence
    request["_artifact_bytes"] = artifact
    request["_validated_challenge"] = challenge
    return invocation, request, challenge_digest


def _split_official_url(raw: Any, host: str, label: str) -> Any:
    value = _require_string(raw, label, 4096)
    try:
        url = urlsplit(value)
        port = url.port
    except ValueError as error:
        raise Rejected(f"{label} is not an absolute URL") from error
    if (
        url.scheme != "https"
        or url.hostname != host
        or port is not None
        or url.username is not None
        or url.password is not None
        or url.query
        or url.fragment
        or not url.path.startswith("/")
        or "//" in url.path
        or "\\" in url.path
    ):
        _reject(f"{label} is outside the exact official PyPI origin")
    return url


def _distribution_identity(filename: str, digest: str) -> tuple[str, str]:
    try:
        Distribution(name=filename, digest=digest)
        if filename.endswith(".whl"):
            parsed_name, parsed_version, _, _ = parse_wheel_filename(filename)
        elif filename.endswith((".tar.gz", ".zip")):
            parsed_name, parsed_version = parse_sdist_filename(filename)
        else:
            _reject("artifact is not a supported wheel or source distribution")
    except (ValueError, TypeError) as error:
        raise Rejected("artifact filename is not a valid Python distribution") from error
    return str(parsed_name), str(parsed_version)


def _validate_urls_and_distribution(request: dict[str, Any]) -> str:
    artifact_url = _split_official_url(
        request["artifact_url"],
        "files.pythonhosted.org",
        "artifact_url",
    )
    artifact_filename = PurePosixPath(artifact_url.path).name
    if not artifact_filename or "%" in artifact_filename:
        _reject("artifact URL filename is empty or ambiguously encoded")
    project, version = _distribution_identity(
        artifact_filename,
        request["artifact_digest"],
    )
    expected_namespace = f"pypi:{canonicalize_name(project)}@"
    if request["_validated_challenge"]["namespace"] != expected_namespace:
        _reject("artifact project does not match the challenge namespace")

    evidence_url = _split_official_url(request["evidence_url"], "pypi.org", "evidence_url")
    parts = evidence_url.path.removeprefix("/").split("/")
    if len(parts) != 5 or parts[0] != "integrity" or parts[4] != "provenance":
        _reject("evidence URL is not an exact PyPI Integrity API provenance route")
    if any(not part or "%" in part for part in parts):
        _reject("evidence URL contains an empty or ambiguously encoded path segment")
    if (
        parts[1] != canonicalize_name(project)
        or parts[2] != version
        or parts[3] != artifact_filename
    ):
        _reject("evidence URL does not identify the exact distribution")
    return artifact_filename


def _safe_archive_path(raw: str) -> tuple[str, ...]:
    if (
        not raw
        or raw != unicodedata.normalize("NFC", raw)
        or "\\" in raw
        or "\x00" in raw
        or raw.startswith("/")
        or re.match(r"^[A-Za-z]:", raw)
        or any(unicodedata.category(character).startswith("C") for character in raw)
    ):
        _reject("archive contains a non-canonical path")
    stripped = raw[:-1] if raw.endswith("/") else raw
    parts = tuple(stripped.split("/"))
    if not parts or any(part in {"", ".", ".."} for part in parts):
        _reject("archive contains an empty or traversing path component")
    return parts


def _record_archive_name(
    names: set[str],
    folded_names: set[str],
    raw: str,
) -> tuple[str, ...]:
    parts = _safe_archive_path(raw)
    canonical = "/".join(parts)
    folded = canonical.casefold()
    if canonical in names or folded in folded_names:
        _reject("archive contains duplicate or case-colliding entries")
    names.add(canonical)
    folded_names.add(folded)
    return parts


def _expected_sdist_root(filename: str) -> str:
    if filename.endswith(".tar.gz"):
        return filename[: -len(".tar.gz")]
    if filename.endswith(".zip"):
        return filename[: -len(".zip")]
    _reject("source distribution suffix is unsupported")


def _validate_challenge_member_path(
    filename: str,
    document: str,
    path: tuple[str, ...],
) -> None:
    if filename.endswith(".whl"):
        if path != (document,):
            _reject("wheel challenge is not at its policy-defined root path")
    elif path != (_expected_sdist_root(filename), document):
        _reject("sdist challenge is not at its policy-defined top-level path")


def _preflight_zip_directory(artifact: bytes, maximum_entries: int) -> int:
    """Count central-directory records before ZipFile allocates one object each."""

    minimum_eocd_bytes = _ZIP_EOCD.size
    if len(artifact) < minimum_eocd_bytes:
        _reject("artifact is not a structurally valid ZIP archive")
    search_start = max(len(artifact) - minimum_eocd_bytes - 0xFFFF, 0)
    eocd_offset = artifact.rfind(b"PK\x05\x06", search_start)
    if eocd_offset < 0 or eocd_offset + minimum_eocd_bytes > len(artifact):
        _reject("artifact is not a structurally valid ZIP archive")
    try:
        (
            _signature,
            disk_number,
            directory_disk,
            entries_on_disk,
            declared_entries,
            directory_size,
            directory_offset,
            comment_size,
        ) = _ZIP_EOCD.unpack_from(artifact, eocd_offset)
    except struct.error as error:
        raise Rejected("ZIP end-of-directory record is truncated") from error
    if eocd_offset + minimum_eocd_bytes + comment_size != len(artifact):
        _reject("ZIP end-of-directory record or comment is ambiguous")

    sentinel = (
        disk_number == 0xFFFF
        or directory_disk == 0xFFFF
        or entries_on_disk == 0xFFFF
        or declared_entries == 0xFFFF
        or directory_size == 0xFFFFFFFF
        or directory_offset == 0xFFFFFFFF
    )
    directory_end = eocd_offset
    if sentinel:
        locator_offset = eocd_offset - _ZIP64_LOCATOR.size
        if locator_offset < 0:
            _reject("ZIP64 end-of-directory locator is missing")
        try:
            locator_signature, locator_disk, relative_offset, disk_count = (
                _ZIP64_LOCATOR.unpack_from(artifact, locator_offset)
            )
        except struct.error as error:
            raise Rejected("ZIP64 end-of-directory locator is truncated") from error
        if locator_signature != b"PK\x06\x07" or locator_disk != 0 or disk_count != 1:
            _reject("ZIP64 archive is split or has an invalid locator")
        zip64_offset = artifact.rfind(b"PK\x06\x06", 0, locator_offset)
        if zip64_offset < 0 or zip64_offset + _ZIP64_EOCD.size > locator_offset:
            _reject("ZIP64 end-of-directory record is missing")
        try:
            (
                _zip64_signature,
                zip64_record_size,
                _created_version,
                _required_version,
                disk_number,
                directory_disk,
                entries_on_disk,
                declared_entries,
                directory_size,
                directory_offset,
            ) = _ZIP64_EOCD.unpack_from(artifact, zip64_offset)
        except struct.error as error:
            raise Rejected("ZIP64 end-of-directory record is truncated") from error
        if (
            zip64_record_size < 44
            or zip64_offset + 12 + zip64_record_size != locator_offset
            or relative_offset > zip64_offset
            or zip64_offset - relative_offset + directory_offset + directory_size
            != zip64_offset
        ):
            _reject("ZIP64 end-of-directory offsets are inconsistent")
        directory_end = zip64_offset

    if (
        disk_number != 0
        or directory_disk != 0
        or entries_on_disk != declared_entries
        or declared_entries == 0
        or declared_entries > maximum_entries
        or directory_size > directory_end
    ):
        _reject("ZIP entry count is empty, split, or exceeds policy")
    directory_start = directory_end - directory_size
    cursor = directory_start
    actual_entries = 0
    while cursor < directory_end:
        if (
            cursor + _ZIP_CENTRAL_HEADER_BYTES > directory_end
            or artifact[cursor : cursor + 4] != b"PK\x01\x02"
        ):
            _reject("ZIP central directory is malformed")
        try:
            name_length, extra_length, entry_comment_length = struct.unpack_from(
                "<HHH",
                artifact,
                cursor + 28,
            )
        except struct.error as error:
            raise Rejected("ZIP central-directory entry is truncated") from error
        record_size = (
            _ZIP_CENTRAL_HEADER_BYTES
            + name_length
            + extra_length
            + entry_comment_length
        )
        if record_size > directory_end - cursor:
            _reject("ZIP central-directory entry extends beyond its boundary")
        actual_entries += 1
        if actual_entries > maximum_entries:
            _reject("ZIP entry count is empty, split, or exceeds policy")
        cursor += record_size
    if actual_entries != declared_entries:
        _reject("ZIP central-directory entry count is inconsistent")
    return actual_entries


def _read_zip_challenge(
    artifact: bytes,
    filename: str,
    document: str,
    archive_policy: dict[str, Any],
) -> bytes:
    expected_entries = _preflight_zip_directory(
        artifact,
        archive_policy["max_entries"],
    )
    try:
        archive = zipfile.ZipFile(io.BytesIO(artifact), "r")
    except (OSError, zipfile.BadZipFile) as error:
        raise Rejected("artifact is not a structurally valid ZIP archive") from error
    with archive:
        infos = archive.infolist()
        if len(infos) != expected_entries:
            _reject("ZIP parser and preflight entry counts disagree")
        names: set[str] = set()
        folded_names: set[str] = set()
        challenge_infos: list[tuple[zipfile.ZipInfo, tuple[str, ...]]] = []
        total = 0
        intervals: list[tuple[int, int]] = []
        for info in infos:
            parts = _record_archive_name(names, folded_names, info.orig_filename)
            mode = (info.external_attr >> 16) & 0xFFFF
            kind = stat.S_IFMT(mode)
            if kind not in {0, stat.S_IFREG, stat.S_IFDIR}:
                _reject("ZIP contains a link or special entry")
            if info.flag_bits & (0x1 | 0x20 | 0x40):
                _reject("ZIP contains encrypted or patched data")
            if info.compress_type not in {zipfile.ZIP_STORED, zipfile.ZIP_DEFLATED}:
                _reject("ZIP compression method is outside policy")
            if info.file_size > archive_policy["max_entry_bytes"]:
                _reject("ZIP entry exceeds the per-entry policy limit")
            total += info.file_size
            if total > archive_policy["max_uncompressed_bytes"]:
                _reject("ZIP uncompressed size exceeds policy")
            if (
                info.file_size
                and info.file_size
                > max(info.compress_size, 1) * archive_policy["max_compression_ratio"]
            ):
                _reject("ZIP entry exceeds the compression-ratio policy")
            if info.is_dir() and (info.file_size != 0 or info.compress_size != 0):
                _reject("ZIP directory entry has data")
            if (
                info.header_offset < 0
                or info.header_offset + 30 > archive.start_dir
            ):
                _reject("ZIP local header offset is outside the archive")
            try:
                (
                    signature,
                    _version,
                    local_flags,
                    local_method,
                    _time,
                    _date,
                    _crc,
                    _compressed_size,
                    _uncompressed_size,
                    name_length,
                    extra_length,
                ) = struct.unpack_from("<IHHHHHIIIHH", artifact, info.header_offset)
            except struct.error as error:
                raise Rejected("ZIP local header is truncated") from error
            local_name_start = info.header_offset + 30
            local_name_end = local_name_start + name_length
            data_start = local_name_end + extra_length
            data_end = data_start + info.compress_size
            if data_end > archive.start_dir:
                _reject("ZIP entry overlaps its central directory")
            try:
                local_name = artifact[local_name_start:local_name_end].decode(
                    "utf-8" if local_flags & 0x800 else "cp437"
                )
            except UnicodeDecodeError as error:
                raise Rejected("ZIP local filename is not decodable") from error
            if (
                signature != 0x04034B50
                or local_flags != info.flag_bits
                or local_method != info.compress_type
                or local_name != info.orig_filename
            ):
                _reject("ZIP local and central headers disagree")
            intervals.append((info.header_offset, data_end))
            if parts[-1] == document:
                challenge_infos.append((info, parts))

        intervals.sort()
        for previous, current in zip(intervals, intervals[1:]):
            if previous[1] > current[0]:
                _reject("ZIP entries overlap")
        if len(challenge_infos) != 1:
            _reject("artifact must contain exactly one challenge document")
        info, path = challenge_infos[0]
        if info.is_dir() or info.file_size > MAX_CHALLENGE_BYTES:
            _reject("challenge entry is not a bounded regular file")
        _validate_challenge_member_path(filename, document, path)
        try:
            with archive.open(info, "r") as handle:
                challenge = handle.read(MAX_CHALLENGE_BYTES + 1)
        except (OSError, RuntimeError, zipfile.BadZipFile) as error:
            raise Rejected("challenge ZIP entry could not be read safely") from error
        if len(challenge) != info.file_size or len(challenge) > MAX_CHALLENGE_BYTES:
            _reject("challenge ZIP entry is truncated or oversized")
        return challenge


class _BoundedReader:
    """Bound reads from a decompressor without materializing its whole output."""

    def __init__(
        self,
        source: Any,
        maximum_bytes: int,
        maximum_ratio_bytes: int,
    ) -> None:
        self._source = source
        self._maximum_bytes = maximum_bytes
        self._maximum_ratio_bytes = maximum_ratio_bytes
        self.count = 0

    def read(self, size: int = -1) -> bytes:
        effective_limit = min(self._maximum_bytes, self._maximum_ratio_bytes)
        allowance = effective_limit + 1 - self.count
        if allowance <= 0:
            _reject("sdist decompression exceeds archive policy")
        bounded_size = allowance if size < 0 else min(size, allowance)
        data = self._source.read(bounded_size)
        self.count += len(data)
        if self.count > self._maximum_bytes:
            _reject("sdist uncompressed size exceeds policy")
        if self.count > self._maximum_ratio_bytes:
            _reject("sdist exceeds the compression-ratio policy")
        return data


def _read_tar_challenge(
    artifact: bytes,
    filename: str,
    document: str,
    archive_policy: dict[str, Any],
) -> bytes:
    try:
        with gzip.GzipFile(fileobj=io.BytesIO(artifact), mode="rb") as compressed:
            bounded = _BoundedReader(
                compressed,
                archive_policy["max_uncompressed_bytes"],
                max(len(artifact), 1) * archive_policy["max_compression_ratio"],
            )
            with tarfile.open(fileobj=bounded, mode="r|") as archive:
                names: set[str] = set()
                folded_names: set[str] = set()
                entry_count = 0
                total = 0
                previous_end = 0
                challenge_count = 0
                challenge: bytes | None = None
                for member in archive:
                    entry_count += 1
                    if entry_count > archive_policy["max_entries"]:
                        _reject("TAR entry count is empty or exceeds policy")
                    parts = _record_archive_name(names, folded_names, member.name)
                    if not (member.isfile() or member.isdir()) or member.issparse():
                        _reject("TAR contains a link, sparse file, or special entry")
                    if member.size > archive_policy["max_entry_bytes"]:
                        _reject("TAR entry exceeds the per-entry policy limit")
                    total += member.size
                    if total > archive_policy["max_uncompressed_bytes"]:
                        _reject("TAR declared size exceeds policy")
                    if member.isfile():
                        end = member.offset_data + member.size
                        if member.offset_data < previous_end:
                            _reject("TAR entries overlap")
                        previous_end = end
                    if parts[-1] == document:
                        challenge_count += 1
                        if not member.isfile() or member.size > MAX_CHALLENGE_BYTES:
                            _reject("challenge TAR entry is not a bounded regular file")
                        _validate_challenge_member_path(filename, document, parts)
                        handle = archive.extractfile(member)
                        if handle is None:
                            _reject("challenge TAR entry is unreadable")
                        with handle:
                            challenge = handle.read(MAX_CHALLENGE_BYTES + 1)
                        if (
                            len(challenge) != member.size
                            or len(challenge) > MAX_CHALLENGE_BYTES
                        ):
                            _reject("challenge TAR entry is truncated or oversized")
            while bounded.read(64 * 1024):
                pass
    except Rejected:
        raise
    except (EOFError, OSError, tarfile.TarError) as error:
        raise Rejected("sdist gzip or TAR stream is invalid") from error
    if entry_count == 0:
        _reject("TAR entry count is empty or exceeds policy")
    if challenge_count != 1 or challenge is None:
        _reject("artifact must contain exactly one challenge document")
    return challenge


def _verify_embedded_challenge(
    request: dict[str, Any],
    filename: str,
    archive_policy: dict[str, Any],
) -> None:
    challenge = request["_validated_challenge"]
    document = challenge["challenge_document"]
    artifact = request["_artifact_bytes"]
    if filename.endswith((".whl", ".zip")):
        embedded = _read_zip_challenge(
            artifact,
            filename,
            document,
            archive_policy,
        )
    elif filename.endswith(".tar.gz"):
        embedded = _read_tar_challenge(
            artifact,
            filename,
            document,
            archive_policy,
        )
    else:
        _reject("artifact archive format is unsupported")
    embedded_value = _parse_json(embedded, "embedded challenge")
    try:
        embedded_canonical = rfc8785.dumps(embedded_value)
        expected_canonical = rfc8785.dumps(challenge)
    except Exception as error:
        raise Rejected("challenge is not representable as RFC 8785 JSON") from error
    if embedded_canonical != expected_canonical:
        _reject("embedded challenge does not exactly match the signed request challenge")


def _validate_evidence_shape(value: Any) -> dict[str, Any]:
    provenance = _require_object_optional(
        value,
        "provenance",
        {"attestation_bundles"},
        {"version"},
    )
    if (
        "version" in provenance
        and (type(provenance["version"]) is not int or provenance["version"] != 1)
    ):
        _reject("PyPI provenance version is unsupported")
    bundles = provenance["attestation_bundles"]
    if not isinstance(bundles, list) or not bundles or len(bundles) > 32:
        _reject("provenance bundle count is empty or exceeds policy")
    for bundle_index, bundle_value in enumerate(bundles):
        bundle = _require_object(
            bundle_value,
            f"provenance.attestation_bundles[{bundle_index}]",
            _BUNDLE_KEYS,
        )
        _validate_publisher_shape(
            bundle["publisher"],
            f"provenance.attestation_bundles[{bundle_index}].publisher",
        )
        attestations = bundle["attestations"]
        if not isinstance(attestations, list) or not attestations or len(attestations) > 2:
            _reject("provenance attestation count is empty or exceeds PEP 740 limits")
        for attestation_index, attestation_value in enumerate(attestations):
            label = (
                f"provenance.attestation_bundles[{bundle_index}]"
                f".attestations[{attestation_index}]"
            )
            attestation = _require_object(
                attestation_value,
                label,
                _ATTESTATION_KEYS,
            )
            if type(attestation["version"]) is not int or attestation["version"] != 1:
                _reject("PEP 740 attestation version is unsupported")
            material = _require_object(
                attestation["verification_material"],
                f"{label}.verification_material",
                _VERIFICATION_MATERIAL_KEYS,
            )
            entries = material["transparency_entries"]
            if not isinstance(entries, list) or len(entries) != 1:
                _reject("attestation must contain exactly one transparency entry")
            _require_object(
                attestation["envelope"],
                f"{label}.envelope",
                _ENVELOPE_KEYS,
            )
    return provenance


def _selected_policy_entry(
    policy_value: dict[str, Any],
    request: dict[str, Any],
) -> tuple[dict[str, Any], dict[str, Any]]:
    matches: list[tuple[dict[str, Any], dict[str, Any]]] = []
    for entry in policy_value["publishers"]:
        if (
            entry["namespace"] == request["_validated_challenge"]["namespace"]
            and entry["issuer"] == request["expected_issuer"]
            and entry["subject"] == request["expected_subject"]
        ):
            normalized, _ = _normal_publisher(entry["publisher"], "policy publisher")
            matches.append((entry, normalized))
    if len(matches) != 1:
        _reject("request publisher identity is not uniquely authorized by policy")
    return matches[0]


def _select_publish_attestation(
    provenance: Any,
    normalized_publisher: dict[str, Any],
) -> tuple[Any, Any]:
    matching_bundles = [
        bundle
        for bundle in provenance.attestation_bundles
        if bundle.publisher.model_dump(exclude_none=True) == normalized_publisher
    ]
    if len(matching_bundles) != 1:
        _reject("evidence does not contain exactly one policy-authorized publisher bundle")
    publish_attestations = []
    for attestation in matching_bundles[0].attestations:
        try:
            statement = _parse_json(attestation.envelope.statement, "attestation statement")
        except Exception as error:
            raise Rejected("attestation statement cannot be inspected") from error
        if isinstance(statement, dict) and statement.get("predicateType") == PUBLISH_PREDICATE:
            publish_attestations.append(attestation)
    if len(publish_attestations) != 1:
        _reject("publisher bundle does not contain exactly one PyPI publish attestation")
    return publish_attestations[0], matching_bundles[0].publisher


def _verify_statement(payload: bytes, filename: str, artifact_digest: str) -> None:
    statement = _require_object(
        _parse_json(payload, "verified DSSE statement"),
        "verified DSSE statement",
        _STATEMENT_KEYS,
    )
    if (
        statement["_type"] != IN_TOTO_STATEMENT
        or statement["predicateType"] != PUBLISH_PREDICATE
        or statement["predicate"] is not None
    ):
        _reject("verified statement is not the exact PyPI publish predicate")
    subjects = statement["subject"]
    if not isinstance(subjects, list) or len(subjects) != 1:
        _reject("verified statement must contain exactly one subject")
    subject = _require_object(subjects[0], "verified statement subject", _SUBJECT_KEYS)
    if subject["name"] != filename or subject["digest"] != {"sha256": artifact_digest}:
        _reject("verified statement subject does not match exact filename and SHA-256")


def _load_trusted_root(raw: bytes) -> TrustedRoot:
    """Parse the exact TrustedRoot bytes that matched the invocation pin."""

    try:
        inner = trustroot_v1.TrustedRoot.from_json(raw)
        return TrustedRoot(inner)
    except Exception as error:
        raise Rejected("pinned Sigstore trusted root is invalid") from error


def _verified_proof_window(bundle: Any, challenge: dict[str, Any], now: int) -> tuple[int, int]:
    """Bind proof issuance to Rekor's verified integration time."""

    certificate = bundle.signing_certificate
    certificate_issued_at = int(certificate.not_valid_before_utc.timestamp())
    certificate_expires_at = int(certificate.not_valid_after_utc.timestamp())
    integrated_time = getattr(bundle.log_entry._inner, "integrated_time", None)
    if type(integrated_time) is not int:
        _reject("verified transparency entry has no canonical integrated time")
    if (
        integrated_time < challenge["issued_at"] - MAX_CLOCK_SKEW_SECONDS
        or integrated_time >= challenge["expires_at"]
        or integrated_time < certificate_issued_at
        or integrated_time > certificate_expires_at
        or certificate_expires_at <= integrated_time
        or certificate_expires_at < challenge["expires_at"]
        or integrated_time > now + MAX_CLOCK_SKEW_SECONDS
        or now >= certificate_expires_at
        or now >= challenge["expires_at"]
    ):
        _reject(
            "transparency integration, certificate, and challenge validity "
            "windows do not overlap safely"
        )
    return integrated_time, certificate_expires_at


def _verify_sigstore(
    request: dict[str, Any],
    policy_value: dict[str, Any],
    root_raw: bytes,
    filename: str,
    now: int,
) -> tuple[int, int]:
    _, normalized_publisher = _selected_policy_entry(policy_value, request)
    evidence_value = _validate_evidence_shape(
        _parse_json(request["_evidence_bytes"], "PyPI provenance")
    )
    try:
        provenance = Provenance.model_validate(evidence_value)
    except Exception as error:
        raise Rejected("evidence is not a valid PEP 740 provenance object") from error
    attestation, publisher = _select_publish_attestation(provenance, normalized_publisher)
    try:
        bundle = attestation.to_bundle()
    except Exception as error:
        raise Rejected("PEP 740 attestation cannot be converted to a Sigstore bundle") from error

    # pypi-attestations' public Attestation.verify API chooses its own
    # production/staging root. CREG must instead use the exact activation-pinned
    # TrustedRoot, so use the same provider-specific Publisher policy conversion
    # that Attestation.verify uses internally, under an exact dependency lock.
    # Missing or changed support fails closed on upgrade.
    publisher_policy_factory = getattr(publisher, "_as_policy", None)
    if not callable(publisher_policy_factory):
        _reject("pypi-attestations publisher policy API is unavailable")
    try:
        publisher_policy = publisher_policy_factory()
    except Exception as error:
        raise Rejected("PyPI publisher identity policy could not be constructed") from error
    identity_policy = sigstore_policy.AllOf(
        [
            publisher_policy,
            sigstore_policy.Identity(identity=request["expected_subject"]),
            sigstore_policy.OIDCIssuerV2(request["expected_issuer"]),
        ]
    )
    trusted_root = _load_trusted_root(root_raw)
    try:
        payload_type, payload = Verifier(trusted_root=trusted_root).verify_dsse(
            bundle,
            identity_policy,
        )
    except Exception as error:
        raise Rejected("offline Sigstore DSSE verification failed") from error
    if payload_type != DSSE_JSON_TYPE:
        _reject("verified DSSE payload type is not in-toto JSON")
    _verify_statement(payload, filename, request["artifact_digest"])

    return _verified_proof_window(
        bundle,
        request["_validated_challenge"],
        now,
    )


def verify_invocation(raw: bytes, executable_directory: Path, *, now: int | None = None) -> bytes:
    """Verify one invocation and return canonical result bytes.

    `executable_directory` is the immutable directory containing the pinned
    executable and its sibling policy/root deployment files.
    """

    verified_at = int(time.time()) if now is None else now
    invocation, request, challenge_digest = _validate_invocation(raw, verified_at)
    policy_path, policy_raw = _load_pinned_file(
        executable_directory,
        POLICY_FILENAME,
        invocation["policy_sha256"],
        MAX_POLICY_BYTES,
        "PyPI authority policy",
    )
    del policy_path
    _, root_raw = _load_pinned_file(
        executable_directory,
        TRUST_ROOT_FILENAME,
        invocation["trust_root_sha256"],
        MAX_TRUST_ROOT_BYTES,
        "Sigstore trusted root",
    )
    policy_value = _validate_policy(policy_raw, invocation)
    filename = _validate_urls_and_distribution(request)
    _verify_embedded_challenge(request, filename, policy_value["archive"])
    proof_issued_at, proof_expires_at = _verify_sigstore(
        request,
        policy_value,
        root_raw,
        filename,
        verified_at,
    )
    challenge = request["_validated_challenge"]
    result = {
        "schema_version": PROTOCOL_VERSION,
        "verified": True,
        "adapter_id": invocation["adapter_id"],
        "adapter_version": invocation["adapter_version"],
        "authority": AUTHORITY,
        "evidence_url": request["evidence_url"],
        "evidence_sha256": request["evidence_sha256"],
        "artifact_url": request["artifact_url"],
        "issuer": request["expected_issuer"],
        "subject": request["expected_subject"],
        "audience": AUDIENCE,
        "purpose": challenge["purpose"],
        "namespace": challenge["namespace"],
        "challenge_sha256": challenge_digest,
        "current_controller_ed25519_pubkey": challenge[
            "current_controller_ed25519_pubkey"
        ],
        "current_controller_evm_address": challenge["current_controller_evm_address"],
        "next_controller_ed25519_pubkey": challenge["next_controller_ed25519_pubkey"],
        "next_controller_evm_address": challenge["next_controller_evm_address"],
        "authority_intent_sha256": challenge_digest,
        "artifact_digest_algorithm": "sha256",
        "artifact_digest": request["artifact_digest"],
        "policy_sha256": invocation["policy_sha256"],
        "trust_root_sha256": invocation["trust_root_sha256"],
        "proof_issued_at": proof_issued_at,
        "proof_expires_at": proof_expires_at,
        "verified_at": verified_at,
    }
    try:
        return rfc8785.dumps(result)
    except Exception as error:
        raise Rejected("verified result cannot be encoded as canonical JSON") from error


def main() -> None:
    """Console-script entry point."""

    try:
        if os.environ.get("CREG_AUTHORITY_ADAPTER_PROTOCOL") != str(PROTOCOL_VERSION):
            _reject("unsupported or missing authority-adapter protocol")
        raw = _read_all_stdin()
        executable_directory = Path(sys.argv[0]).resolve().parent
        output = verify_invocation(raw, executable_directory)
    except Rejected as error:
        print(f"pypi authority verification rejected: {error}", file=sys.stderr)
        raise SystemExit(1) from None
    except Exception:
        print("pypi authority verification rejected: internal fail-closed error", file=sys.stderr)
        raise SystemExit(1) from None
    sys.stdout.buffer.write(output)
    sys.stdout.buffer.flush()


if __name__ == "__main__":
    main()
