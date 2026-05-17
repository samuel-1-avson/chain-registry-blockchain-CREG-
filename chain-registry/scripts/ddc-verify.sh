#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage: scripts/ddc-verify.sh

Required environment:
  TRUSTED_ENV        Command prefix for the diverse trusted build path.
  BUILD_PARENT       Script that builds the parent compiler source. Receives: <stage1-dir>
  BUILD_CUT          Script that rebuilds the compiler-under-test. Receives: <stage2-dir>
  RELEASED_COMPILER  Path to the released compiler/generator binary to verify.

Optional environment:
  OUT_DIR            Evidence directory. Default: artifacts/ddc-evidence
  STAGE1_COMPILER    Overridden automatically after BUILD_PARENT unless already set.

This is intentionally generic. Chain Registry only treats owned compiler-like
tools and code generators as direct DDC targets; runtime binaries use
scripts/release-assurance.sh for reproducible rebuild verification.
USAGE
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  usage
  exit 0
fi

: "${TRUSTED_ENV:?set TRUSTED_ENV to a trusted build command prefix}"
: "${BUILD_PARENT:?set BUILD_PARENT to the parent compiler build script}"
: "${BUILD_CUT:?set BUILD_CUT to the compiler-under-test build script}"
: "${RELEASED_COMPILER:?set RELEASED_COMPILER to the released compiler binary}"

if [[ ! -x "$BUILD_PARENT" ]]; then
  echo "BUILD_PARENT is not executable: $BUILD_PARENT" >&2
  exit 2
fi

if [[ ! -x "$BUILD_CUT" ]]; then
  echo "BUILD_CUT is not executable: $BUILD_CUT" >&2
  exit 2
fi

if [[ ! -f "$RELEASED_COMPILER" ]]; then
  echo "RELEASED_COMPILER does not exist: $RELEASED_COMPILER" >&2
  exit 2
fi

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$script_dir/.." && pwd)"
out_dir="${OUT_DIR:-$repo_root/artifacts/ddc-evidence}"
mkdir -p "$out_dir"

workdir="$(mktemp -d "${TMPDIR:-/tmp}/creg-ddc.XXXXXX")"
cleanup() {
  if [[ -n "${workdir:-}" && -d "$workdir" && "$(basename "$workdir")" == creg-ddc.* ]]; then
    rm -rf "$workdir"
  fi
}
trap cleanup EXIT

stage1="$workdir/stage1"
stage2="$workdir/stage2"
mkdir -p "$stage1" "$stage2"

sha256_file="$out_dir/sha256.txt"
evidence_file="$out_dir/ddc-evidence.json"
diff_file="$out_dir/diffoscope.txt"

hash_file() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

echo "[1/4] Building parent compiler through trusted path"
# TRUSTED_ENV is a command prefix by design, e.g. "nix develop .#ddc --command".
# shellcheck disable=SC2086
$TRUSTED_ENV "$BUILD_PARENT" "$stage1"

stage1_compiler="${STAGE1_COMPILER:-$stage1/compiler}"
if [[ ! -f "$stage1_compiler" ]]; then
  echo "Stage-one compiler was not produced at $stage1_compiler" >&2
  exit 1
fi

echo "[2/4] Rebuilding compiler-under-test"
STAGE1_COMPILER="$stage1_compiler" "$BUILD_CUT" "$stage2"

rebuilt_compiler="$stage2/compiler"
if [[ ! -f "$rebuilt_compiler" ]]; then
  echo "Rebuilt compiler was not produced at $rebuilt_compiler" >&2
  exit 1
fi

released_hash="$(hash_file "$RELEASED_COMPILER")"
rebuilt_hash="$(hash_file "$rebuilt_compiler")"
{
  printf '%s  %s\n' "$released_hash" "$RELEASED_COMPILER"
  printf '%s  %s\n' "$rebuilt_hash" "$rebuilt_compiler"
} > "$sha256_file"

echo "[3/4] Comparing rebuilt compiler to released compiler"
result="pass"
if ! cmp -s "$RELEASED_COMPILER" "$rebuilt_compiler"; then
  result="fail"
  echo "DDC mismatch. Hashes written to $sha256_file" >&2
  if command -v diffoscope >/dev/null 2>&1; then
    diffoscope "$RELEASED_COMPILER" "$rebuilt_compiler" > "$diff_file" || true
    echo "diffoscope output written to $diff_file" >&2
  fi
fi

cat > "$evidence_file" <<JSON
{
  "schema_version": 1,
  "kind": "ddc",
  "result": "$result",
  "source": "deep-research-report.md",
  "git_commit": "$(git rev-parse HEAD 2>/dev/null || echo unknown)",
  "trusted_env": "${TRUSTED_ENV}",
  "build_parent": "${BUILD_PARENT}",
  "build_cut": "${BUILD_CUT}",
  "released_compiler": "${RELEASED_COMPILER}",
  "released_sha256": "$released_hash",
  "rebuilt_sha256": "$rebuilt_hash",
  "sha256_file": "$sha256_file",
  "diffoscope_file": "$(if [[ -f "$diff_file" ]]; then echo "$diff_file"; fi)"
}
JSON

echo "[4/4] Evidence written to $evidence_file"
if [[ "$result" != "pass" ]]; then
  exit 1
fi

echo "DDC PASS"
