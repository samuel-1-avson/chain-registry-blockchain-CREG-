# syntax=docker/dockerfile:1.7
# Unified Reproducible Multi-Stage Build for Chain Registry Validator & Services
# (ADR-018 Increment 7 / VNA-26 — single canonical container recipe).
#
# Targets:
#   - creg-gateway        : Zone 1 DMZ HTTP gateway (UID 10001:10001)
#   - creg-consensusd     : Zone 2 P2P + PBFT consensus daemon (UID 10002:10002)
#   - creg-sandboxd       : Zone 3 kernel microVM / nsjail executor (UID 10003:10003)
#   - creg-agent-runner   : Zone 4 CaMeL AI deep investigator (UID 10004:10004)
#   - secure-node         : Four-zone bundle + supervisor + nsjail + verified rootfs (UID 10002:10002)
#   - recovery-inspector  : Phase-C RocksDB safety & state inspector (UID 10002:10002)
#   - node-runtime        : Phase-C / fleet validator runtime with nsjail (UID 10002:10002)
#   - node-cli            : Phase-C validator + CLI bundle (UID 10002:10002)
#   - runtime             : (default) Full service bundle with nsjail (UID 10002:10002)

ARG CREG_SPEC_SIGNING_PUBKEY=3fe87833177b04b0a18600eace041692a35fb2b85f6e14e6734d894a0fd61c01
ARG CREG_NODE_FEATURES=phase-c-real-fleet
ARG CREG_CLI_FEATURES=common/phase-c-canonical-transactions
ARG NSJAIL_VERSION=3.4
ARG SOURCE_REVISION=""
ARG SOURCE_MANIFEST_SHA256=""
ARG SOURCE_DATE_EPOCH=0

# ── Stage 1: Pinned Rust 1.98.1 Toolchain ────────────────────────────────────
FROM rust:1.98.1-slim-bookworm AS rust-toolchain

# ── Stage 2: Pinned nsjail 3.4 Builder (Ubuntu 24.04 glibc 2.39) ─────────────
FROM ubuntu:24.04@sha256:a853f94d226358a79c740cfc7bce0c289748f3fe3488d921d038ccd752c61b60 AS nsjail-builder
ARG NSJAIL_VERSION
RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
    ca-certificates \
    build-essential \
    bison \
    flex \
    libprotobuf-dev \
    protobuf-compiler \
    libnl-route-3-dev \
    git \
    pkg-config \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /build/nsjail
RUN git clone --depth 1 --branch "${NSJAIL_VERSION}" https://github.com/google/nsjail.git . && \
    make -j"$(nproc)"

# ── Stage 3: Minimal Alpine Sandbox Rootfs Builders (VNA-14) ─────────────────
FROM node:20-alpine AS rootfs-npm
RUN apk add --no-cache ca-certificates && \
    rm -rf /var/cache/apk/* /tmp/* /root/.cache && \
    adduser -D -u 99999 sandbox && \
    mkdir -p /pkg /home/sandbox && \
    chown 99999:99999 /pkg /home/sandbox && \
    find / -xdev -perm /6000 -type f -exec chmod a-s {} + 2>/dev/null || true

FROM python:3.12-alpine AS rootfs-pip
RUN apk add --no-cache ca-certificates && \
    rm -rf /var/cache/apk/* /tmp/* /root/.cache && \
    adduser -D -u 99999 sandbox && \
    mkdir -p /pkg /home/sandbox && \
    chown 99999:99999 /pkg /home/sandbox && \
    find / -xdev -perm /6000 -type f -exec chmod a-s {} + 2>/dev/null || true

FROM rust:1.98.1-alpine AS rootfs-cargo
RUN apk add --no-cache ca-certificates musl-dev && \
    rm -rf /var/cache/apk/* /tmp/* /root/.cache && \
    adduser -D -u 99999 sandbox && \
    mkdir -p /pkg /home/sandbox && \
    chown 99999:99999 /pkg /home/sandbox && \
    find / -xdev -perm /6000 -type f -exec chmod a-s {} + 2>/dev/null || true

# Assemble sandbox rootfs tree and compute CREG-ROOTFS-TREE-V1 digests.
FROM ubuntu:24.04@sha256:a853f94d226358a79c740cfc7bce0c289748f3fe3488d921d038ccd752c61b60 AS rootfs-bundle
RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
    python3 \
    && rm -rf /var/lib/apt/lists/*
COPY --from=rootfs-npm   / /rootfs-out/npm/
COPY --from=rootfs-pip   / /rootfs-out/pip/
COPY --from=rootfs-cargo / /rootfs-out/cargo/
RUN ROOTFS_DIR=/rootfs-out python3 - <<'PY'
import hashlib, json, os, stat

root = os.environ["ROOTFS_DIR"]
ecosystems = ["npm", "pip", "cargo"]
manifest = {"schema_version": 1, "ecosystems": {}}

for eco in ecosystems:
    eco_dir = os.path.join(root, eco)
    entries = []
    for cur, dirs, files in os.walk(eco_dir, followlinks=False):
        rel_cur = os.path.relpath(cur, eco_dir).replace("\\", "/")
        for d in dirs:
            p = os.path.join(cur, d)
            rel = d if rel_cur == "." else f"{rel_cur}/{d}"
            entries.append((rel, p))
        for f in files:
            p = os.path.join(cur, f)
            rel = f if rel_cur == "." else f"{rel_cur}/{f}"
            entries.append((rel, p))
    entries.sort(key=lambda x: x[0])
    h = hashlib.sha256()
    h.update(b"CREG-ROOTFS-TREE-V1\n")
    reg_count = 0
    for rel, p in entries:
        st = os.lstat(p)
        if stat.S_ISLNK(st.st_mode):
            target = os.readlink(p).replace("\\", "/")
            h.update(f"l {rel} {target}\n".encode("utf-8"))
        elif stat.S_ISDIR(st.st_mode):
            h.update(f"d {rel}\n".encode("utf-8"))
        elif stat.S_ISREG(st.st_mode):
            reg_count += 1
            fh = hashlib.sha256()
            size = 0
            with open(p, "rb") as fp:
                while True:
                    chunk = fp.read(65536)
                    if not chunk:
                        break
                    size += len(chunk)
                    fh.update(chunk)
            h.update(f"f {rel} {size} {fh.hexdigest()}\n".encode("utf-8"))
    manifest["ecosystems"][eco] = {
        "tree_sha256": h.hexdigest(),
        "regular_file_count": reg_count,
    }

with open(os.path.join(root, "rootfs-digests.json"), "w", encoding="utf-8") as out:
    json.dump(manifest, out, indent=2, sort_keys=True)
    out.write("\n")
PY

# ── Stage 4: Native Rust Workspace Builder (Ubuntu 24.04 + Rust 1.98.1) ──────
FROM ubuntu:24.04@sha256:a853f94d226358a79c740cfc7bce0c289748f3fe3488d921d038ccd752c61b60 AS builder
ARG CREG_SPEC_SIGNING_PUBKEY
ARG CREG_NODE_FEATURES
ARG CREG_CLI_FEATURES
ARG SOURCE_REVISION
ARG SOURCE_MANIFEST_SHA256
ARG SOURCE_DATE_EPOCH

RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
    ca-certificates \
    curl \
    pkg-config \
    libssl-dev \
    build-essential \
    clang-17 \
    llvm-17 \
    llvm-17-dev \
    libclang-17-dev \
    libclang1-17 \
    cmake \
    libopenblas-dev \
    protobuf-compiler \
    libprotobuf-dev \
    git \
    && rm -rf /var/lib/apt/lists/*

RUN ln -sf /usr/bin/clang-17 /usr/bin/clang && \
    ln -sf /usr/bin/llvm-config-17 /usr/bin/llvm-config

COPY --from=rust-toolchain /usr/local/cargo /usr/local/cargo
COPY --from=rust-toolchain /usr/local/rustup /usr/local/rustup

ENV CARGO_HOME=/usr/local/cargo \
    RUSTUP_HOME=/usr/local/rustup \
    PATH="/usr/local/cargo/bin:$PATH" \
    LIBCLANG_PATH=/usr/lib/llvm-17/lib \
    LLVM_CONFIG_PATH=/usr/bin/llvm-config-17 \
    CLANG_PATH=/usr/bin/clang-17 \
    CARGO_REGISTRIES_CRATES_IO_PROTOCOL=sparse \
    CARGO_BUILD_JOBS=2 \
    CARGO_INCREMENTAL=0 \
    CREG_SPEC_SIGNING_PUBKEY=${CREG_SPEC_SIGNING_PUBKEY} \
    SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH}

RUN mkdir -p "$CARGO_HOME" && \
    printf '[net]\nretry = 10\ngit-fetch-with-cli = true\n\n[http]\nmultiplexing = false\ntimeout = 600\n\n[registries.crates-io]\nprotocol = "sparse"\n' > "$CARGO_HOME/config.toml"

RUN cargo --version && rustc --version && \
    rustc --version | grep '^rustc 1\.98\.1 ' && \
    test -e "${LIBCLANG_PATH}/libclang.so.1" -o -e "${LIBCLANG_PATH}/libclang.so"

WORKDIR /build

# Workspace crate manifests for deterministic locked fetch.
COPY Cargo.toml Cargo.lock ./
COPY crates/cli/Cargo.toml                  crates/cli/
COPY crates/common/Cargo.toml               crates/common/
COPY crates/common/build.rs                 crates/common/
COPY crates/common/proto/                   crates/common/proto/
COPY crates/consensus/Cargo.toml            crates/consensus/
COPY crates/indexer/Cargo.toml              crates/indexer/
COPY crates/ai-config/Cargo.toml crates/ai-config/
COPY crates/dev-cli/Cargo.toml crates/dev-cli/
COPY crates/registry-view/Cargo.toml crates/registry-view/
COPY crates/faucet/Cargo.toml               crates/faucet/
COPY crates/gateway/Cargo.toml              crates/gateway/
COPY crates/insurance/Cargo.toml            crates/insurance/
COPY crates/ipfs-pinner/Cargo.toml          crates/ipfs-pinner/
COPY crates/l1-tx/Cargo.toml                crates/l1-tx/
COPY crates/ml-validator/Cargo.toml         crates/ml-validator/
COPY crates/node/Cargo.toml                 crates/node/
COPY crates/node/build.rs                   crates/node/
COPY crates/relayer/Cargo.toml              crates/relayer/
COPY crates/resolver/Cargo.toml             crates/resolver/
COPY crates/secrets/Cargo.toml              crates/secrets/
COPY crates/threshold-encryption/Cargo.toml crates/threshold-encryption/
COPY crates/unpack/Cargo.toml               crates/unpack/
COPY crates/validator/Cargo.toml            crates/validator/
COPY crates/wasm-sandbox/Cargo.toml         crates/wasm-sandbox/
COPY crates/zone-ipc/Cargo.toml             crates/zone-ipc/

# Stub workspace targets so `cargo fetch --locked` caches cleanly before full source copy.
RUN mkdir -p crates/cli/src                  && echo "fn main() {}"      > crates/cli/src/main.rs && \
    mkdir -p crates/common/src               && echo "pub fn _stub() {}" > crates/common/src/lib.rs && \
    mkdir -p crates/consensus/src            && echo "pub fn _stub() {}" > crates/consensus/src/lib.rs && \
    mkdir -p crates/ai-config/src && echo "pub fn _stub() {}" > crates/ai-config/src/lib.rs && \
    mkdir -p crates/dev-cli/src && echo "fn main() {}" > crates/dev-cli/src/main.rs && \
    mkdir -p crates/registry-view/src && echo "pub fn _stub() {}" > crates/registry-view/src/lib.rs && \
    mkdir -p crates/indexer/src              && echo "pub fn _stub() {}" > crates/indexer/src/lib.rs && \
    mkdir -p crates/faucet/src               && echo "fn main() {}"      > crates/faucet/src/main.rs && \
    mkdir -p crates/gateway/src              && echo "fn main() {}"      > crates/gateway/src/main.rs && \
    mkdir -p crates/insurance/src            && echo "pub fn _stub() {}" > crates/insurance/src/lib.rs && \
    mkdir -p crates/ipfs-pinner/src          && echo "pub fn _stub() {}" > crates/ipfs-pinner/src/lib.rs && \
    mkdir -p crates/l1-tx/src                && echo "pub fn _stub() {}" > crates/l1-tx/src/lib.rs && \
    mkdir -p crates/ml-validator/src         && echo "pub fn _stub() {}" > crates/ml-validator/src/lib.rs && \
    mkdir -p crates/node/src/bin             && echo "pub fn _stub() {}" > crates/node/src/lib.rs && \
    echo "fn main() {}" > crates/node/src/main.rs && \
    echo "fn main() {}" > crates/node/src/bin/dump_openapi.rs && \
    echo "fn main() {}" > crates/node/src/bin/creg_consensusd.rs && \
    echo "fn main() {}" > crates/node/src/bin/creg_node_supervisor.rs && \
    echo "fn main() {}" > crates/node/src/bin/creg_authority_rejection_fixture.rs && \
    echo "fn main() {}" > crates/node/src/bin/creg_recovery_inspect.rs && \
    mkdir -p crates/relayer/src              && echo "fn main() {}"      > crates/relayer/src/main.rs && \
    mkdir -p crates/resolver/src             && echo "pub fn _stub() {}" > crates/resolver/src/lib.rs && \
    mkdir -p crates/secrets/src              && echo "pub fn _stub() {}" > crates/secrets/src/lib.rs && \
    mkdir -p crates/threshold-encryption/src && echo "pub fn _stub() {}" > crates/threshold-encryption/src/lib.rs && \
    mkdir -p crates/unpack/src               && echo "pub fn _stub() {}" > crates/unpack/src/lib.rs && \
    mkdir -p crates/validator/src/bin        && echo "pub fn _stub() {}" > crates/validator/src/lib.rs && \
    echo "fn main() {}" > crates/validator/src/bin/creg_sandboxd.rs && \
    echo "fn main() {}" > crates/validator/src/bin/creg_agent_runner.rs && \
    mkdir -p crates/wasm-sandbox/src         && echo "pub fn _stub() {}" > crates/wasm-sandbox/src/lib.rs && \
    mkdir -p crates/zone-ipc/src             && echo "pub fn _stub() {}" > crates/zone-ipc/src/lib.rs

RUN cargo fetch --locked || (sleep 5 && cargo fetch --locked) || (sleep 10 && cargo fetch --locked)

# Copy tracked compiler & runtime inputs (no circuits/ or models/ bloat).
COPY crates/ crates/
COPY migrations/ migrations/
COPY config/ config/
COPY rules/ rules/
COPY validators/ validators/
COPY testnet/ testnet/
COPY docker-compose*.yml ./

# Optional committed-source manifest verification (Phase-C reproducible builds).
COPY BUILD-SOURCE-MANIFEST.json* BUILD-SOURCE-FILES.sha256* ./
# The checksum list covers every archived file, including this recipe, so the
# recipe must be present in /build for `sha256sum -c` to pass.
COPY Dockerfile ./
RUN if [ -n "${SOURCE_MANIFEST_SHA256}" ]; then \
        test -n "${SOURCE_REVISION}" && \
        printf '%s  BUILD-SOURCE-MANIFEST.json\n' "${SOURCE_MANIFEST_SHA256}" | sha256sum -c - && \
        sha256sum --quiet -c BUILD-SOURCE-FILES.sha256; \
    else \
        printf '{"schema":"creg-canonical-dockerfile-v2","rust_toolchain":"1.98.1"}\n' > BUILD-SOURCE-MANIFEST.json; \
    fi

# Build Zone 1 (`creg-gateway`), Zone 2/Supervisor/Node (`chain-registry-node`),
# Zone 3/4 (`validator`), CLI (`creg`), Faucet, and Relayer binaries.
RUN env -u RUSTFLAGS -u CARGO_ENCODED_RUSTFLAGS \
    LIBCLANG_PATH=/usr/lib/llvm-17/lib \
    LLVM_CONFIG_PATH=/usr/bin/llvm-config-17 \
    CLANG_PATH=/usr/bin/clang-17 \
    RUST_BACKTRACE=1 \
    cargo build --release --locked \
        --package chain-registry-node \
        --bin creg-node \
        --bin creg-consensusd \
        --bin creg-node-supervisor \
        --bin creg_recovery_inspect \
        --features "${CREG_NODE_FEATURES}" && \
    cargo build --release --locked --package creg-indexer && \
    cargo build --release --locked \
        --package validator \
        --bin creg-sandboxd \
        --bin creg-agent-runner && \
    cargo build --release --locked \
        --package creg-gateway \
        --bin creg-gateway && \
    cargo build --release --locked \
        --package chain-registry-cli \
        --bin creg \
        --features "${CREG_CLI_FEATURES}" && \
    cargo build --release --locked \
        --package faucet \
        --bin faucet && \
    cargo build --release --locked \
        --package chain-registry-relayer \
        --bin creg-relayer && \
    /build/target/release/creg chain-spec validate \
        /build/testnet/phase-c/r7/chain-spec.json \
        --signature /build/testnet/phase-c/r7/chain-spec.json.sig \
        --pubkey "${CREG_SPEC_SIGNING_PUBKEY}" --require-signature

# ── Stage 5: Shared Hardened Runtime Base (Ubuntu 24.04, no host container daemon) ──
FROM ubuntu:24.04@sha256:a853f94d226358a79c740cfc7bce0c289748f3fe3488d921d038ccd752c61b60 AS runtime-base
ARG CREG_SPEC_SIGNING_PUBKEY
ARG CREG_NODE_FEATURES
ARG CREG_CLI_FEATURES
ARG SOURCE_REVISION
ARG SOURCE_MANIFEST_SHA256
ARG SOURCE_DATE_EPOCH

LABEL org.opencontainers.image.revision="${SOURCE_REVISION}" \
      org.opencontainers.image.base.name="ubuntu:24.04" \
      org.opencontainers.image.base.digest="sha256:a853f94d226358a79c740cfc7bce0c289748f3fe3488d921d038ccd752c61b60" \
      dev.creg.source-manifest.sha256="${SOURCE_MANIFEST_SHA256}" \
      dev.creg.source-date-epoch="${SOURCE_DATE_EPOCH}" \
      dev.creg.rust-version="1.98.1" \
      dev.creg.node-features="${CREG_NODE_FEATURES}" \
      dev.creg.cli-features="${CREG_CLI_FEATURES}" \
      dev.creg.spec-signing-pubkey="${CREG_SPEC_SIGNING_PUBKEY}"

RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
    ca-certificates \
    curl \
    libssl3 \
    libopenblas0 \
    libnl-route-3-200 \
    libprotobuf32t64 \
    strace \
    && rm -rf /var/lib/apt/lists/*

# Create isolated four-zone service principals matching docker-compose.validator-v2.yml.
RUN groupadd -g 10001 creg-gateway      && useradd -r -u 10001 -g 10001 -s /usr/sbin/nologin creg-gateway && \
    groupadd -g 10002 creg-consensus    && useradd -r -u 10002 -g 10002 -s /usr/sbin/nologin creg-consensus && \
    groupadd -g 10003 creg-sandbox      && useradd -r -u 10003 -g 10003 -s /usr/sbin/nologin creg-sandbox && \
    groupadd -g 10004 creg-agent        && useradd -r -u 10004 -g 10004 -s /usr/sbin/nologin creg-agent && \
    mkdir -p /app /data /var/lib/creg /var/lib/creg-rootfs /run/creg-ipc /etc/creg && \
    chown 10002:10002 /app /data /var/lib/creg /etc/creg && \
    chown 10003:10003 /var/lib/creg-rootfs

WORKDIR /app
COPY --from=builder /build/BUILD-SOURCE-MANIFEST.json /app/BUILD-SOURCE-MANIFEST.json

# ── Target: Zone 1 (`creg-gateway`, UID 10001:10001) ─────────────────────────
FROM runtime-base AS creg-gateway
COPY --from=builder --chown=10001:10001 /build/target/release/creg-gateway /app/creg-gateway
USER 10001:10001
EXPOSE 8080 18080
ENTRYPOINT ["/app/creg-gateway"]

# ── Target: Zone 2 (`creg-consensusd`, UID 10002:10002) ──────────────────────
FROM runtime-base AS creg-consensusd
COPY --from=builder --chown=10002:10002 /build/target/release/creg-consensusd /app/creg-consensusd
COPY --from=builder --chown=10002:10002 /build/config /app/config
COPY --from=builder --chown=10002:10002 /build/testnet/chain-spec.sepolia.json /etc/creg/chain-spec.json
ENV CREG_DATA_DIR=/var/lib/creg \
    CREG_PBFT_VIEW_CHANGE_READY=true
USER 10002:10002
EXPOSE 9000
ENTRYPOINT ["/app/creg-consensusd"]

# ── Target: Zone 3 (`creg-sandboxd`, UID 10003:10003) ────────────────────────
FROM runtime-base AS creg-sandboxd
COPY --from=nsjail-builder /build/nsjail/nsjail /usr/bin/nsjail
COPY --from=builder --chown=10003:10003 /build/target/release/creg-sandboxd /app/creg-sandboxd
COPY --from=builder --chown=10003:10003 /build/config/sandbox /app/config/sandbox
COPY --from=builder --chown=10003:10003 /build/rules /app/rules
COPY --from=builder --chown=10003:10003 /build/validators /app/validators
COPY --from=rootfs-bundle --chown=10003:10003 /rootfs-out /var/lib/creg-rootfs
COPY --from=rootfs-bundle --chown=10003:10003 /rootfs-out /app/config/sandbox/rootfs
ENV CREG_SANDBOX_ROOTFS=/var/lib/creg-rootfs
USER 10003:10003
ENTRYPOINT ["/app/creg-sandboxd"]

# ── Target: Zone 4 (`creg-agent-runner`, UID 10004:10004) ────────────────────
FROM runtime-base AS creg-agent-runner
COPY --from=builder --chown=10004:10004 /build/target/release/creg-agent-runner /app/creg-agent-runner
COPY --from=builder --chown=10004:10004 /build/config/validator-ai.example.toml /etc/creg/validator-ai.toml
COPY --from=builder --chown=10004:10004 /build/config/validator-ai.example.toml /etc/creg/validator-ai.example.toml
USER 10004:10004
ENTRYPOINT ["/app/creg-agent-runner"]

# ── Target: `recovery-inspector` (Phase-C RocksDB & safety inspector) ────────
FROM runtime-base AS creg-indexer
COPY --from=builder --chown=10002:10002 /build/target/release/creg-indexer /app/creg-indexer
USER 10002:10002
ENTRYPOINT ["/app/creg-indexer"]

# ── Target: `faucet-runtime` (public faucet, ADR-019 claim signing) ───────────
# UID 65532 matches the deployed faucet container; the service keeps no local
# state (PostgreSQL only), so the image needs no writable volume.
FROM runtime-base AS faucet-runtime
COPY --from=builder --chown=65532:65532 /build/target/release/faucet /app/faucet
USER 65532:65532
EXPOSE 8082
ENTRYPOINT ["/app/faucet"]

# ── Target: `relayer-runtime` (ADR-019 l1-tx relayer) ────────────────────────
# Needs a writable RELAYER_DATA_DIR for the sled journal; /data is created and
# owned by 10002 in runtime-base.
FROM runtime-base AS relayer-runtime
COPY --from=builder --chown=10002:10002 /build/target/release/creg-relayer /app/creg-relayer
USER 10002:10002
EXPOSE 8083
ENTRYPOINT ["/app/creg-relayer"]

FROM runtime-base AS recovery-inspector
COPY --from=builder --chown=10002:10002 /build/target/release/creg_recovery_inspect /app/creg_recovery_inspect
USER 10002:10002
ENTRYPOINT ["/app/creg_recovery_inspect"]

# ── Target: `node-runtime` (Phase-C validator node runtime with nsjail) ──────
FROM runtime-base AS node-runtime
COPY --from=nsjail-builder /build/nsjail/nsjail /usr/bin/nsjail
COPY --from=builder --chown=10002:10002 /build/target/release/creg-node /app/creg-node
COPY --from=builder --chown=10002:10002 /build/config /app/config
COPY --from=builder --chown=10002:10002 /build/rules /app/rules
COPY --from=builder --chown=10002:10002 /build/validators /app/validators
COPY --from=rootfs-bundle --chown=10002:10002 /rootfs-out /app/config/sandbox/rootfs
ENV CREG_DATA_DIR=/data \
    CREG_PBFT_VIEW_CHANGE_READY=true
USER 10002:10002
ENTRYPOINT ["/app/creg-node"]

# ── Target: `node-cli` (Phase-C validator node + creg CLI) ───────────────────
FROM node-runtime AS node-cli
COPY --from=builder --chown=10002:10002 /build/target/release/creg /app/creg
USER 10002:10002
ENTRYPOINT ["/app/creg-node"]

# ── Target: `secure-node` (Supervised 4-zone validator + nsjail + rootfs) ────
FROM node-cli AS secure-node
COPY --from=builder --chown=10002:10002 /build/target/release/creg-node-supervisor /app/creg-node-supervisor
COPY --from=builder --chown=10002:10002 /build/target/release/creg-consensusd /app/creg-consensusd
COPY --from=builder --chown=10002:10002 /build/target/release/creg-sandboxd /app/creg-sandboxd
COPY --from=builder --chown=10002:10002 /build/target/release/creg-agent-runner /app/creg-agent-runner
COPY --from=builder --chown=10002:10002 /build/target/release/creg-gateway /app/creg-gateway
USER 10002:10002
VOLUME ["/data"]
EXPOSE 8080 9000 50051
HEALTHCHECK --interval=20s --timeout=10s --retries=3 \
  CMD curl -f http://localhost:8080/v1/health || exit 1
ENTRYPOINT ["/app/creg-node"]

# ── Target: `runtime` (Default full service bundle) ──────────────────────────
FROM secure-node AS runtime
COPY --from=builder --chown=10002:10002 /build/target/release/creg-indexer /app/creg-indexer
COPY --from=builder --chown=10002:10002 /build/target/release/faucet /app/faucet
COPY --from=builder --chown=10002:10002 /build/target/release/creg-relayer /app/creg-relayer
USER 10002:10002
ENTRYPOINT ["/app/creg-node"]
