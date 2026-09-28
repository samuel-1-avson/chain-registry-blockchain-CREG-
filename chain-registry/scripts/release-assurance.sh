#!/usr/bin/env bash
# Rebuild release binaries twice and verify byte-identical outputs.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ART="${ROOT}/artifacts/release-assurance"

usage() {
  cat <<'EOF'
Usage: release-assurance.sh [--help]

Build chain-registry-cli (creg) twice with the R7 canonical transaction
feature and deterministic release flags. Outputs must be byte-identical.

Evidence is written to artifacts/release-assurance/.
EOF
}

if [[ "${1:-}" == "--help" || "${1:-}" == "-h" ]]; then
  usage
  exit 0
fi

cd "$ROOT"
mkdir -p "$ART"

export CARGO_INCREMENTAL=0
if [[ -z "${CREG_SPEC_SIGNING_PUBKEY:-}" || ! "${CREG_SPEC_SIGNING_PUBKEY}" =~ ^[0-9a-fA-F]{64}$ ]]; then
  echo "release-assurance: CREG_SPEC_SIGNING_PUBKEY must be a 32-byte hexadecimal public key" >&2
  exit 1
fi
if [[ -z "${SOURCE_DATE_EPOCH:-}" ]]; then
  export SOURCE_DATE_EPOCH="$(git log -1 --pretty=%ct)"
fi
export RUSTFLAGS="${RUSTFLAGS:-} --remap-path-prefix=${ROOT}=/creg"

build_into() {
  local label="$1"
  local target_dir="$2"
  # Reuse the exact same target path for both builds. Different target roots
  # can leak into build-script metadata even with source path remapping; a
  # clean rebuild at one canonical path tests reproducibility without that
  # path-dependent noise.
  rm -rf "$target_dir"
  CARGO_TARGET_DIR="$target_dir" cargo build --locked --release \
    --package chain-registry-cli --bin creg \
    --features common/phase-c-canonical-transactions
  install -m 0755 "$target_dir/release/creg" "$ART/creg-${label}"
}

echo "release-assurance: build A (SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH})"
TARGET_DIR="${ROOT}/target-release-assurance"
build_into a "$TARGET_DIR"
echo "release-assurance: build B"
build_into b "$TARGET_DIR"

if ! cmp -s "$ART/creg-a" "$ART/creg-b"; then
  echo "release-assurance: creg binaries differ between builds" >&2
  exit 1
fi
{
  echo "SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH}"
  sha256sum "$ART/creg-a"
} > "$ART/sha256sum.txt"

echo "release-assurance: OK — reproducible release binaries verified"
cat "$ART/sha256sum.txt"
