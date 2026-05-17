#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage: scripts/release-assurance.sh [--out-dir DIR] [--target PACKAGE:BIN ...]

Builds release binaries twice in isolated Cargo target directories and fails if
the resulting artifacts are not byte-identical. This implements the
reproducible-build release gate from deep-research-report.md.

Defaults:
  --target chain-registry-node:creg-node
  --target chain-registry-cli:creg

Environment:
  SOURCE_DATE_EPOCH  Defaults to the timestamp of HEAD.
  RUSTFLAGS          Additional flags are appended after path remapping.
USAGE
}

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$script_dir/.." && pwd)"
out_dir="$repo_root/artifacts/release-assurance"
targets=("chain-registry-node:creg-node" "chain-registry-cli:creg")
explicit_targets=()

while [[ $# -gt 0 ]]; do
  case "$1" in
    --out-dir)
      out_dir="$2"
      shift 2
      ;;
    --target)
      explicit_targets+=("$2")
      shift 2
      ;;
    --help|-h)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

if [[ ${#explicit_targets[@]} -gt 0 ]]; then
  targets=("${explicit_targets[@]}")
fi

cd "$repo_root"
mkdir -p "$out_dir/binaries"

workdir="$(mktemp -d "${TMPDIR:-/tmp}/creg-release-assurance.XXXXXX")"
cleanup() {
  if [[ -n "${workdir:-}" && -d "$workdir" && "$(basename "$workdir")" == creg-release-assurance.* ]]; then
    rm -rf "$workdir"
  fi
}
trap cleanup EXIT

source_date_epoch="${SOURCE_DATE_EPOCH:-$(git log -1 --pretty=%ct 2>/dev/null || date +%s)}"
export SOURCE_DATE_EPOCH="$source_date_epoch"
export CARGO_INCREMENTAL=0
export RUSTFLAGS="--remap-path-prefix=$repo_root=/src ${RUSTFLAGS:-}"

build_once() {
  local target_dir="$1"
  shift
  export CARGO_TARGET_DIR="$target_dir"
  for target in "$@"; do
    local package="${target%%:*}"
    local bin="${target#*:}"
    echo "Building $package:$bin in $target_dir"
    cargo build --release --locked --package "$package" --bin "$bin"
  done
}

hash_file() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

target_a="$workdir/target-a"
target_b="$workdir/target-b"
build_once "$target_a" "${targets[@]}"
build_once "$target_b" "${targets[@]}"

sha_file="$out_dir/sha256.txt"
evidence_file="$out_dir/release-assurance.json"
: > "$sha_file"

all_pass=true
target_json=""
for target in "${targets[@]}"; do
  package="${target%%:*}"
  bin="${target#*:}"
  ext=""
  if [[ "${OS:-}" == "Windows_NT" ]]; then
    ext=".exe"
  fi
  artifact_a="$target_a/release/$bin$ext"
  artifact_b="$target_b/release/$bin$ext"

  if [[ ! -f "$artifact_a" || ! -f "$artifact_b" ]]; then
    echo "Missing artifact for $package:$bin" >&2
    exit 1
  fi

  hash_a="$(hash_file "$artifact_a")"
  hash_b="$(hash_file "$artifact_b")"
  printf '%s  %s\n' "$hash_a" "$artifact_a" >> "$sha_file"
  printf '%s  %s\n' "$hash_b" "$artifact_b" >> "$sha_file"

  status="pass"
  if ! cmp -s "$artifact_a" "$artifact_b"; then
    status="fail"
    all_pass=false
    echo "Reproducible build mismatch for $package:$bin" >&2
    if command -v diffoscope >/dev/null 2>&1; then
      diffoscope "$artifact_a" "$artifact_b" > "$out_dir/diffoscope-$bin.txt" || true
    fi
  fi

  cp "$artifact_a" "$out_dir/binaries/$bin$ext"
  if [[ -n "$target_json" ]]; then
    target_json="$target_json,"
  fi
  target_json="$target_json
    {
      \"package\": \"$package\",
      \"binary\": \"$bin$ext\",
      \"status\": \"$status\",
      \"sha256\": \"$hash_a\"
    }"
done

rustc_version="$(rustc --version 2>/dev/null || echo unknown)"
cargo_version="$(cargo --version 2>/dev/null || echo unknown)"
git_commit="$(git rev-parse HEAD 2>/dev/null || echo unknown)"

cat > "$evidence_file" <<JSON
{
  "schema_version": 1,
  "kind": "release-assurance",
  "source": "deep-research-report.md",
  "result": "$(if [[ "$all_pass" == true ]]; then echo pass; else echo fail; fi)",
  "git_commit": "$git_commit",
  "source_date_epoch": "$source_date_epoch",
  "rustc_version": "$rustc_version",
  "cargo_version": "$cargo_version",
  "rustflags": "$RUSTFLAGS",
  "sha256_file": "$sha_file",
  "targets": [$target_json
  ]
}
JSON

echo "Release assurance evidence written to $evidence_file"
echo "Verified binaries copied to $out_dir/binaries"

if [[ "$all_pass" != true ]]; then
  exit 1
fi
