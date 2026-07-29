"""Build, smoke-test, and hash the self-contained authority verifier."""

from __future__ import annotations

import argparse
import hashlib
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path


PROJECT_ROOT = Path(__file__).resolve().parents[1]
SPEC_PATH = PROJECT_ROOT / "pypi-verifier.spec"
ARTIFACT_NAME = "creg-pypi-authority-verifier.exe" if os.name == "nt" else (
    "creg-pypi-authority-verifier"
)
BUILD_PYTHON = (3, 11, 14)
SOURCE_DATE_EPOCH = "1735689600"


def _safe_remove_tree(path: Path, expected_parent: Path) -> None:
    resolved = path.resolve()
    if resolved.parent != expected_parent.resolve():
        raise RuntimeError(f"refusing to remove unexpected build path: {resolved}")
    if resolved.exists():
        shutil.rmtree(resolved)


def _build(label: str) -> Path:
    work_path = PROJECT_ROOT / "build" / label
    dist_path = PROJECT_ROOT / "build" / f"{label}-dist"
    _safe_remove_tree(work_path, PROJECT_ROOT / "build")
    _safe_remove_tree(dist_path, PROJECT_ROOT / "build")
    work_path.parent.mkdir(parents=True, exist_ok=True)
    command = [
        sys.executable,
        "-m",
        "PyInstaller",
        "--clean",
        "--noconfirm",
        "--workpath",
        str(work_path),
        "--distpath",
        str(dist_path),
        str(SPEC_PATH),
    ]
    subprocess.run(command, cwd=PROJECT_ROOT, check=True)
    artifact = dist_path / ARTIFACT_NAME
    if not artifact.is_file():
        raise RuntimeError(f"PyInstaller did not produce {artifact}")
    return artifact


def _sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def _runtime_environment(runtime_directory: Path) -> dict[str, str]:
    if not runtime_directory.is_absolute():
        raise RuntimeError("runtime scratch directory must be absolute")
    value = str(runtime_directory)
    return {
        "CREG_AUTHORITY_ADAPTER_PROTOCOL": "2",
        "TEMP": value,
        "TMP": value,
        "TMPDIR": value,
    }


def _smoke_test(path: Path) -> None:
    scratch_root = PROJECT_ROOT / "build" / "smoke-runtime"
    scratch_root.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(
        prefix="invocation-",
        dir=scratch_root,
    ) as temporary:
        runtime_directory = Path(temporary).resolve()
        runtime_directory.chmod(0o700)
        child = subprocess.run(
            [str(path)],
            input=b"{}",
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            cwd=path.parent,
            env=_runtime_environment(runtime_directory),
            timeout=15,
            check=False,
        )
    if child.returncode == 0:
        raise RuntimeError("invalid invocation unexpectedly verified")
    if child.stdout:
        raise RuntimeError("rejection wrote bytes to stdout")
    expected = b"pypi authority verification rejected:"
    if not child.stderr.startswith(expected):
        raise RuntimeError(
            "frozen verifier did not reach its fail-closed protocol handler; "
            f"stderr={child.stderr!r}"
        )


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--verify-reproducible",
        action="store_true",
        help="perform two clean builds and require identical executable bytes",
    )
    args = parser.parse_args()

    if sys.version_info[:3] != BUILD_PYTHON:
        expected = ".".join(str(part) for part in BUILD_PYTHON)
        raise SystemExit(f"build requires CPython {expected} exactly")
    if os.environ.get("PYTHONHASHSEED") != "0":
        environment = os.environ.copy()
        environment["PYTHONHASHSEED"] = "0"
        environment["SOURCE_DATE_EPOCH"] = SOURCE_DATE_EPOCH
        raise SystemExit(
            subprocess.run(
                [sys.executable, str(Path(__file__).resolve()), *sys.argv[1:]],
                cwd=PROJECT_ROOT,
                env=environment,
                check=False,
            ).returncode
        )
    if os.environ.get("SOURCE_DATE_EPOCH") != SOURCE_DATE_EPOCH:
        raise SystemExit(
            f"SOURCE_DATE_EPOCH must equal the reviewed value {SOURCE_DATE_EPOCH}"
        )

    first = _build("onefile-a")
    first_digest = _sha256(first)
    artifact = first
    if args.verify_reproducible:
        second = _build("onefile-b")
        second_digest = _sha256(second)
        if first_digest != second_digest:
            raise RuntimeError(
                "clean one-file builds were not reproducible: "
                f"{first_digest} != {second_digest}"
            )
        artifact = second

    _smoke_test(artifact)
    output_directory = PROJECT_ROOT / "dist"
    output_directory.mkdir(parents=True, exist_ok=True)
    output = output_directory / ARTIFACT_NAME
    shutil.copyfile(artifact, output)
    output.chmod(artifact.stat().st_mode)
    print(f"artifact={output}")
    print(f"sha256={_sha256(output)}")
    print("private_temp_cleared_environment_smoke=passed")
    if args.verify_reproducible:
        print("same_host_clean_rebuild=identical")


if __name__ == "__main__":
    main()
