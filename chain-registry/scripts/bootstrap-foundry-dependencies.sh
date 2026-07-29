#!/usr/bin/env bash
set -euo pipefail

# Install the exact contract dependencies used by CI into the ignored Foundry
# library directory. Tags are documentation only; immutable commit IDs are the
# authority and are checked after checkout.
FORGE_STD_REPOSITORY="https://github.com/foundry-rs/forge-std.git"
FORGE_STD_COMMIT="0844d7e1fc5e60d77b68e469bff60265f236c398"
OPENZEPPELIN_REPOSITORY="https://github.com/OpenZeppelin/openzeppelin-contracts.git"
OPENZEPPELIN_COMMIT="dc44c9f1a4c3b10af99492eed84f83ed244203f6"

project_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
library_root="${project_root}/contracts/lib"
mkdir -p "${library_root}"

install_dependency() {
  local name="$1"
  local repository="$2"
  local commit="$3"
  local destination="${library_root}/${name}"

  if [[ -e "${destination}" ]]; then
    local installed
    installed="$(git -C "${destination}" rev-parse HEAD 2>/dev/null || true)"
    if [[ "${installed}" != "${commit}" ]]; then
      echo "${destination} exists but is not the pinned ${commit}" >&2
      return 1
    fi
    return 0
  fi

  local staging
  staging="$(mktemp -d "${library_root}/.${name}.XXXXXX")"
  git -C "${staging}" init --quiet
  git -C "${staging}" remote add origin "${repository}"
  git -C "${staging}" fetch --quiet --depth 1 origin "${commit}"
  git -C "${staging}" checkout --quiet --detach FETCH_HEAD

  local installed
  installed="$(git -C "${staging}" rev-parse HEAD)"
  if [[ "${installed}" != "${commit}" ]]; then
    echo "fetched ${name} at ${installed}, expected ${commit}" >&2
    return 1
  fi

  mv "${staging}" "${destination}"
}

install_dependency "forge-std" "${FORGE_STD_REPOSITORY}" "${FORGE_STD_COMMIT}"
install_dependency \
  "openzeppelin-contracts" \
  "${OPENZEPPELIN_REPOSITORY}" \
  "${OPENZEPPELIN_COMMIT}"

echo "Foundry dependencies match the repository pins."
