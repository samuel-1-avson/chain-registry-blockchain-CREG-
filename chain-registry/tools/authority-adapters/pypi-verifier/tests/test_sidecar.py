from __future__ import annotations

import hashlib
import io
import json
import os
import runpy
import subprocess
import sys
import tarfile
import tempfile
import unittest
import warnings
import zipfile
from datetime import datetime, timezone
from pathlib import Path
from types import SimpleNamespace
from unittest import mock

import rfc8785
from cryptography.hazmat.primitives import serialization
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey

from creg_pypi_verifier import sidecar


NOW = 1_900_000_000
FILENAME = "demo-1.0.0-py3-none-any.whl"
ISSUER = "https://token.actions.githubusercontent.com"
SUBJECT = (
    "https://github.com/example/demo/.github/workflows/release.yml@refs/tags/v1.0.0"
)


def challenge() -> dict[str, object]:
    private_key = Ed25519PrivateKey.from_private_bytes(bytes([7]) * 32)
    public_key = private_key.public_key().public_bytes(
        serialization.Encoding.Raw,
        serialization.PublicFormat.Raw,
    )
    value: dict[str, object] = {
        "schema_version": 1,
        "purpose": "claim",
        "challenge_document": "creg-claim.json",
        "chain_id": "creg-testnet-1",
        "genesis_hash": "44" * 32,
        "namespace": "pypi:demo@",
        "current_controller_ed25519_pubkey": "",
        "current_controller_evm_address": "",
        "next_controller_ed25519_pubkey": public_key.hex(),
        "next_controller_evm_address": "0x" + "88" * 20,
        "nonce": 7,
        "issued_at": NOW - 10,
        "expires_at": NOW + 300,
        "next_controller_signature": "",
    }
    intent = (
        "creg-namespace-authority-intent-v1\n"
        "service:creg-package-ownership\n"
        "purpose:claim\n"
        "document:creg-claim.json\n"
        "chain_id:creg-testnet-1\n"
        f"genesis_hash:{'44' * 32}\n"
        "namespace:pypi:demo@\n"
        "current_ed25519_pubkey:\n"
        "current_evm_address:\n"
        f"next_ed25519_pubkey:{public_key.hex()}\n"
        f"next_evm_address:{'0x' + '88' * 20}\n"
        "nonce:7\n"
        f"issued_at:{NOW - 10}\n"
        f"expires_at:{NOW + 300}"
    ).encode()
    value["next_controller_signature"] = private_key.sign(intent).hex()
    return value


def archive_policy() -> dict[str, int]:
    return {
        "max_entries": 100,
        "max_entry_bytes": 1024 * 1024,
        "max_uncompressed_bytes": 8 * 1024 * 1024,
        "max_compression_ratio": 100,
    }


def policy() -> dict[str, object]:
    return {
        "schema_version": 1,
        "authority": "pypi_trusted_publisher",
        "adapter_id": "creg-pypi-sigstore",
        "adapter_version": "1.0.0",
        "sigstore_instance": "production",
        "predicate_type": sidecar.PUBLISH_PREDICATE,
        "publishers": [
            {
                "namespace": "pypi:demo@",
                "issuer": ISSUER,
                "subject": SUBJECT,
                "publisher": {
                    "kind": "GitHub",
                    "repository": "example/demo",
                    "workflow": "release.yml",
                },
            }
        ],
        "archive": archive_policy(),
    }


def wheel(entries: list[tuple[str, bytes]]) -> bytes:
    output = io.BytesIO()
    with warnings.catch_warnings():
        warnings.simplefilter("ignore", UserWarning)
        with zipfile.ZipFile(output, "w", compression=zipfile.ZIP_DEFLATED) as archive:
            for name, contents in entries:
                archive.writestr(name, contents)
    return output.getvalue()


def invocation(artifact: bytes, evidence: bytes = b"{}") -> dict[str, object]:
    return {
        "schema_version": 2,
        "adapter_id": "creg-pypi-sigstore",
        "adapter_version": "1.0.0",
        "policy_sha256": "11" * 32,
        "trust_root_sha256": "22" * 32,
        "request": {
            "schema_version": 2,
            "authority": "pypi_trusted_publisher",
            "evidence_url": (
                "https://pypi.org/integrity/demo/1.0.0/"
                f"{FILENAME}/provenance"
            ),
            "evidence_sha256": hashlib.sha256(evidence).hexdigest(),
            "evidence_hex": evidence.hex(),
            "artifact_url": (
                "https://files.pythonhosted.org/packages/aa/bb/" + FILENAME
            ),
            "artifact_digest_algorithm": "sha256",
            "artifact_digest": hashlib.sha256(artifact).hexdigest(),
            "artifact_hex": artifact.hex(),
            "expected_issuer": ISSUER,
            "expected_subject": SUBJECT,
            "expected_audience": "pypi",
            "challenge": challenge(),
        },
    }


class ChallengeTests(unittest.TestCase):
    def test_signed_challenge_is_bound_to_rust_intent_format(self) -> None:
        value, digest = sidecar._validate_challenge(challenge(), NOW)
        self.assertEqual(value["namespace"], "pypi:demo@")
        self.assertRegex(digest, r"^[0-9a-f]{64}$")

    def test_tampered_challenge_signature_fails_closed(self) -> None:
        value = challenge()
        value["next_controller_evm_address"] = "0x" + "99" * 20
        with self.assertRaisesRegex(sidecar.Rejected, "signature verification"):
            sidecar._validate_challenge(value, NOW)

    def test_non_jcs_safe_nonce_is_rejected(self) -> None:
        value = challenge()
        value["nonce"] = sidecar.MAX_SAFE_INTEGER + 1
        with self.assertRaisesRegex(sidecar.Rejected, "integer range"):
            sidecar._validate_challenge(value, NOW)


class ArchiveTests(unittest.TestCase):
    def test_wheel_reads_exactly_one_root_challenge_without_extraction(self) -> None:
        expected = rfc8785.dumps(challenge())
        artifact = wheel(
            [
                ("creg-claim.json", expected),
                ("demo/__init__.py", b""),
            ]
        )
        actual = sidecar._read_zip_challenge(
            artifact,
            FILENAME,
            "creg-claim.json",
            archive_policy(),
        )
        self.assertEqual(actual, expected)

    def test_duplicate_challenge_is_rejected(self) -> None:
        expected = rfc8785.dumps(challenge())
        artifact = wheel(
            [
                ("creg-claim.json", expected),
                ("other/creg-claim.json", expected),
            ]
        )
        with self.assertRaisesRegex(sidecar.Rejected, "exactly one challenge"):
            sidecar._read_zip_challenge(
                artifact,
                FILENAME,
                "creg-claim.json",
                archive_policy(),
            )

    def test_zip_entry_limit_is_enforced_before_zipfile_allocation(self) -> None:
        expected = rfc8785.dumps(challenge())
        artifact = wheel(
            [("creg-claim.json", expected)]
            + [(f"demo/{index}", b"") for index in range(100)]
        )
        restrictive_policy = archive_policy()
        restrictive_policy["max_entries"] = 100
        with (
            mock.patch.object(
                sidecar.zipfile,
                "ZipFile",
                side_effect=AssertionError("ZipFile was constructed"),
            ),
            self.assertRaisesRegex(sidecar.Rejected, "entry count"),
        ):
            sidecar._read_zip_challenge(
                artifact,
                FILENAME,
                "creg-claim.json",
                restrictive_policy,
            )

    def test_traversing_zip_member_is_rejected(self) -> None:
        artifact = wheel(
            [
                ("creg-claim.json", rfc8785.dumps(challenge())),
                ("../escape.py", b""),
            ]
        )
        with self.assertRaisesRegex(sidecar.Rejected, "traversing"):
            sidecar._read_zip_challenge(
                artifact,
                FILENAME,
                "creg-claim.json",
                archive_policy(),
            )

    def test_nul_truncated_zip_challenge_name_is_rejected(self) -> None:
        artifact = wheel(
            [("creg-claim.jsonxx", rfc8785.dumps(challenge()))]
        ).replace(
            b"creg-claim.jsonxx",
            b"creg-claim.json\x00x",
        )
        with self.assertRaisesRegex(sidecar.Rejected, "non-canonical path"):
            sidecar._read_zip_challenge(
                artifact,
                FILENAME,
                "creg-claim.json",
                archive_policy(),
            )

    def test_zip_symlink_is_rejected(self) -> None:
        output = io.BytesIO()
        with zipfile.ZipFile(output, "w") as archive:
            archive.writestr("creg-claim.json", rfc8785.dumps(challenge()))
            link = zipfile.ZipInfo("demo/link")
            link.create_system = 3
            link.external_attr = (0o120777 << 16) | 0xA000
            archive.writestr(link, b"target")
        with self.assertRaisesRegex(sidecar.Rejected, "link or special"):
            sidecar._read_zip_challenge(
                output.getvalue(),
                FILENAME,
                "creg-claim.json",
                archive_policy(),
            )

    def test_sdist_hard_link_is_rejected(self) -> None:
        output = io.BytesIO()
        root = "demo-1.0.0"
        with tarfile.open(fileobj=output, mode="w:gz") as archive:
            data = rfc8785.dumps(challenge())
            challenge_info = tarfile.TarInfo(f"{root}/creg-claim.json")
            challenge_info.size = len(data)
            archive.addfile(challenge_info, io.BytesIO(data))
            link = tarfile.TarInfo(f"{root}/link")
            link.type = tarfile.LNKTYPE
            link.linkname = f"{root}/creg-claim.json"
            archive.addfile(link)
        with self.assertRaisesRegex(sidecar.Rejected, "link"):
            sidecar._read_tar_challenge(
                output.getvalue(),
                "demo-1.0.0.tar.gz",
                "creg-claim.json",
                archive_policy(),
            )

    def test_sdist_reads_exactly_one_top_level_challenge(self) -> None:
        output = io.BytesIO()
        root = "demo-1.0.0"
        expected = rfc8785.dumps(challenge())
        with tarfile.open(fileobj=output, mode="w:gz") as archive:
            challenge_info = tarfile.TarInfo(f"{root}/creg-claim.json")
            challenge_info.size = len(expected)
            archive.addfile(challenge_info, io.BytesIO(expected))
            module = tarfile.TarInfo(f"{root}/demo.py")
            module.size = 0
            archive.addfile(module, io.BytesIO())
        actual = sidecar._read_tar_challenge(
            output.getvalue(),
            "demo-1.0.0.tar.gz",
            "creg-claim.json",
            archive_policy(),
        )
        self.assertEqual(actual, expected)

    def test_sdist_decompression_limit_is_enforced_while_streaming(self) -> None:
        output = io.BytesIO()
        root = "demo-1.0.0"
        with tarfile.open(fileobj=output, mode="w:gz") as archive:
            data = rfc8785.dumps(challenge())
            challenge_info = tarfile.TarInfo(f"{root}/creg-claim.json")
            challenge_info.size = len(data)
            archive.addfile(challenge_info, io.BytesIO(data))
            for index in range(150):
                padding_info = tarfile.TarInfo(f"{root}/padding-{index}")
                padding_info.size = 0
                archive.addfile(padding_info, io.BytesIO())
        restrictive_policy = archive_policy()
        restrictive_policy["max_entries"] = 200
        restrictive_policy["max_entry_bytes"] = 64 * 1024
        restrictive_policy["max_uncompressed_bytes"] = 64 * 1024
        restrictive_policy["max_compression_ratio"] = 1000
        with self.assertRaisesRegex(sidecar.Rejected, "uncompressed size"):
            sidecar._read_tar_challenge(
                output.getvalue(),
                "demo-1.0.0.tar.gz",
                "creg-claim.json",
                restrictive_policy,
            )

    def test_embedded_challenge_must_match_request_semantically(self) -> None:
        different = challenge()
        different["nonce"] = 8
        artifact = wheel([("creg-claim.json", rfc8785.dumps(different))])
        request = invocation(artifact)["request"]
        request["_artifact_bytes"] = artifact
        request["_validated_challenge"] = challenge()
        with self.assertRaisesRegex(sidecar.Rejected, "does not exactly match"):
            sidecar._verify_embedded_challenge(
                request,
                FILENAME,
                archive_policy(),
            )


class PolicyAndProtocolTests(unittest.TestCase):
    def test_stdin_framing_carries_rust_hard_max_request_shape(self) -> None:
        value = invocation(b"", b"")
        value["adapter_id"] = '"' * 128
        value["adapter_version"] = "\\" * 64
        request = value["request"]
        assert isinstance(request, dict)
        evidence_prefix = "https://pypi.org/"
        artifact_prefix = "https://files.pythonhosted.org/"
        request["evidence_url"] = evidence_prefix + "a" * (
            4096 - len(evidence_prefix)
        )
        request["artifact_url"] = artifact_prefix + "a" * (
            4096 - len(artifact_prefix)
        )
        request["expected_issuer"] = '"' * 512
        request["expected_subject"] = "\\" * 1024
        challenge_value = request["challenge"]
        assert isinstance(challenge_value, dict)
        challenge_value.update(
            {
                "purpose": "recover",
                "challenge_document": "creg-recovery.json",
                "chain_id": '"' * 128,
                "namespace": "pypi:" + "a" * 506 + "@",
                "current_controller_ed25519_pubkey": "a" * 64,
                "current_controller_evm_address": "0x" + "a" * 40,
                "nonce": sidecar.MAX_SAFE_INTEGER,
                "issued_at": sidecar.MAX_SAFE_INTEGER - 900,
                "expires_at": sidecar.MAX_SAFE_INTEGER,
                "next_controller_signature": "a" * 128,
            }
        )

        # Empty payload strings retain their JSON quotes, so adding the exact
        # maximum hex character counts gives the full canonical shape without
        # allocating either large binary input.
        bounded_metadata_bytes = len(rfc8785.dumps(value))
        hard_max_shape_bytes = bounded_metadata_bytes + 2 * (
            sidecar.HARD_MAX_EVIDENCE_BYTES + sidecar.HARD_MAX_ARTIFACT_BYTES
        )
        self.assertLessEqual(
            bounded_metadata_bytes,
            sidecar.MAX_JCS_INVOCATION_OVERHEAD_BYTES,
        )
        # Special methods are resolved on the type, so use a tiny local class
        # rather than allocating a bytes object of the simulated size.
        sized_input = type(
            "SizedInput",
            (),
            {"__len__": lambda self: hard_max_shape_bytes},
        )()
        buffer = mock.Mock()
        buffer.read.return_value = sized_input
        with mock.patch.object(
            sidecar.sys,
            "stdin",
            SimpleNamespace(buffer=buffer),
        ):
            self.assertIs(sidecar._read_all_stdin(), sized_input)
        buffer.read.assert_called_once_with(sidecar.MAX_STDIN_BYTES + 1)
        sidecar._enforce_stdin_size(sidecar.MAX_STDIN_BYTES)
        with self.assertRaisesRegex(sidecar.Rejected, "protocol limit"):
            sidecar._enforce_stdin_size(sidecar.MAX_STDIN_BYTES + 1)

    def test_policy_must_be_canonical_and_match_adapter_pin(self) -> None:
        invocation_value = invocation(
            wheel([("creg-claim.json", rfc8785.dumps(challenge()))])
        )
        parsed = sidecar._validate_policy(
            rfc8785.dumps(policy()),
            invocation_value,
        )
        self.assertEqual(parsed["sigstore_instance"], "production")
        pretty = json.dumps(policy(), indent=2).encode()
        with self.assertRaisesRegex(sidecar.Rejected, "not canonical"):
            sidecar._validate_policy(pretty, invocation_value)

    def test_unpinned_request_publisher_is_rejected(self) -> None:
        artifact = wheel([("creg-claim.json", rfc8785.dumps(challenge()))])
        request = invocation(artifact)["request"]
        request["_validated_challenge"] = challenge()
        request["expected_subject"] = "https://attacker.invalid/workflow"
        with self.assertRaisesRegex(sidecar.Rejected, "not uniquely authorized"):
            sidecar._selected_policy_entry(policy(), request)

    def test_locked_pypi_publishers_expose_provider_specific_policies(self) -> None:
        publishers = [
            {
                "kind": "GitHub",
                "repository": "example/demo",
                "workflow": "release.yml",
            },
            {
                "kind": "GitLab",
                "repository": "example/demo",
                "workflow_filepath": ".gitlab-ci.yml",
            },
            {
                "kind": "Google",
                "email": "publisher@example.iam.gserviceaccount.com",
            },
            {
                "kind": "CircleCI",
                "project_id": "11111111-1111-1111-1111-111111111111",
                "pipeline_definition_id": "22222222-2222-2222-2222-222222222222",
            },
        ]
        for value in publishers:
            with self.subTest(kind=value["kind"]):
                _, publisher = sidecar._normal_publisher(value, "publisher")
                factory = getattr(publisher, "_as_policy", None)
                self.assertTrue(callable(factory))
                self.assertIsNotNone(factory())

    def test_integrity_and_artifact_urls_bind_same_exact_distribution(self) -> None:
        artifact = wheel([("creg-claim.json", rfc8785.dumps(challenge()))])
        request = invocation(artifact)["request"]
        request["_validated_challenge"] = challenge()
        self.assertEqual(sidecar._validate_urls_and_distribution(request), FILENAME)
        request["evidence_url"] = request["evidence_url"].replace(FILENAME, "other.whl")
        with self.assertRaisesRegex(sidecar.Rejected, "exact distribution"):
            sidecar._validate_urls_and_distribution(request)

    def test_digest_substitution_fails_before_policy_or_sigstore(self) -> None:
        artifact = wheel([("creg-claim.json", rfc8785.dumps(challenge()))])
        value = invocation(artifact)
        value["request"]["artifact_digest"] = "00" * 32
        with self.assertRaisesRegex(sidecar.Rejected, "exact artifact bytes"):
            sidecar._validate_invocation(rfc8785.dumps(value), NOW)

    def test_duplicate_json_member_is_rejected(self) -> None:
        with self.assertRaisesRegex(sidecar.Rejected, "duplicate"):
            sidecar._parse_json(b'{"version":1,"version":1}', "evidence")

    def test_nonfinite_json_number_is_rejected(self) -> None:
        with self.assertRaisesRegex(sidecar.Rejected, "non-finite"):
            sidecar._parse_json(b'{"version":NaN}', "evidence")

    def test_boolean_policy_schema_version_is_rejected(self) -> None:
        value = policy()
        value["schema_version"] = True
        with self.assertRaisesRegex(sidecar.Rejected, "schema version"):
            sidecar._validate_policy(
                rfc8785.dumps(value),
                invocation(wheel([("creg-claim.json", rfc8785.dumps(challenge()))])),
            )

    def test_missing_sibling_material_never_writes_stdout(self) -> None:
        environment = os.environ.copy()
        environment["CREG_AUTHORITY_ADAPTER_PROTOCOL"] = "2"
        environment["PYTHONPATH"] = str(
            Path(__file__).resolve().parents[1] / "src"
        )
        child = subprocess.run(
            [sys.executable, "-m", "creg_pypi_verifier.sidecar"],
            input=b"{}",
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            env=environment,
            check=False,
        )
        self.assertNotEqual(child.returncode, 0)
        self.assertEqual(child.stdout, b"")
        self.assertIn(b"rejected", child.stderr)

    def test_frozen_build_smoke_environment_is_minimal_and_private(self) -> None:
        project_root = Path(__file__).resolve().parents[1]
        build_script = runpy.run_path(project_root / "scripts" / "build_onefile.py")
        with tempfile.TemporaryDirectory() as temporary:
            runtime_directory = Path(temporary).resolve()
            environment = build_script["_runtime_environment"](runtime_directory)
        self.assertEqual(
            environment,
            {
                "CREG_AUTHORITY_ADAPTER_PROTOCOL": "2",
                "TEMP": str(runtime_directory),
                "TMP": str(runtime_directory),
                "TMPDIR": str(runtime_directory),
            },
        )
        with self.assertRaisesRegex(RuntimeError, "must be absolute"):
            build_script["_runtime_environment"](Path("relative"))

    def test_policy_pin_mismatch_fails_before_sigstore(self) -> None:
        artifact = wheel([("creg-claim.json", rfc8785.dumps(challenge()))])
        value = invocation(artifact)
        raw_policy = rfc8785.dumps(policy())
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            (directory / sidecar.POLICY_FILENAME).write_bytes(raw_policy)
            (directory / sidecar.TRUST_ROOT_FILENAME).write_bytes(b"{}")
            with self.assertRaisesRegex(sidecar.Rejected, "policy.*pin"):
                sidecar.verify_invocation(
                    rfc8785.dumps(value),
                    directory,
                    now=NOW,
                )

    def test_trusted_root_parser_consumes_the_exact_pinned_bytes(self) -> None:
        raw = b'{"exact":"pinned bytes"}'
        inner = object()
        trusted_root = object()
        with (
            mock.patch.object(
                sidecar.trustroot_v1.TrustedRoot,
                "from_json",
                return_value=inner,
            ) as parse,
            mock.patch.object(sidecar, "TrustedRoot", return_value=trusted_root) as wrap,
        ):
            actual = sidecar._load_trusted_root(raw)
        self.assertIs(actual, trusted_root)
        parse.assert_called_once_with(raw)
        wrap.assert_called_once_with(inner)

    def test_invalid_pinned_trusted_root_fails_closed(self) -> None:
        with self.assertRaisesRegex(sidecar.Rejected, "trusted root is invalid"):
            sidecar._load_trusted_root(b"{}")

    def test_proof_issuance_uses_verified_transparency_integration_time(self) -> None:
        certificate_start = NOW - 120
        integrated_time = NOW - 5
        certificate_end = NOW + 600
        bundle = SimpleNamespace(
            signing_certificate=SimpleNamespace(
                not_valid_before_utc=datetime.fromtimestamp(
                    certificate_start,
                    timezone.utc,
                ),
                not_valid_after_utc=datetime.fromtimestamp(
                    certificate_end,
                    timezone.utc,
                ),
            ),
            log_entry=SimpleNamespace(
                _inner=SimpleNamespace(integrated_time=integrated_time)
            ),
        )
        self.assertEqual(
            sidecar._verified_proof_window(bundle, challenge(), NOW),
            (integrated_time, certificate_end),
        )

    def test_transparency_integration_after_challenge_expiry_is_rejected(self) -> None:
        value = challenge()
        bundle = SimpleNamespace(
            signing_certificate=SimpleNamespace(
                not_valid_before_utc=datetime.fromtimestamp(
                    NOW - 120,
                    timezone.utc,
                ),
                not_valid_after_utc=datetime.fromtimestamp(
                    NOW + 600,
                    timezone.utc,
                ),
            ),
            log_entry=SimpleNamespace(
                _inner=SimpleNamespace(integrated_time=value["expires_at"])
            ),
        )
        with self.assertRaisesRegex(sidecar.Rejected, "transparency integration"):
            sidecar._verified_proof_window(bundle, value, NOW)


@unittest.skipUnless(
    os.environ.get("CREG_PYPI_PRODUCTION_FIXTURE_DIR"),
    "requires fresh, independently approved production policy/root/package evidence",
)
class ProductionCryptographicFixtureTests(unittest.TestCase):
    def test_real_pypi_publish_attestation_end_to_end(self) -> None:
        directory = Path(os.environ["CREG_PYPI_PRODUCTION_FIXTURE_DIR"]).resolve()
        raw = (directory / "authority-adapter-invocation-v2.jcs.json").read_bytes()
        output = sidecar.verify_invocation(raw, directory)
        parsed = json.loads(output)
        self.assertTrue(parsed["verified"])
        self.assertEqual(parsed["authority"], "pypi_trusted_publisher")
        self.assertEqual(rfc8785.dumps(parsed), output)


if __name__ == "__main__":
    unittest.main()
