# Multi-stage build — produces a minimal runtime image (~80 MB).
# Updated for Chain Registry v0.3.0 with TUI Explorer and Enhanced Web UI

# ── Stage 1: Build Frontend ───────────────────────────────────────────────────
FROM node:20-slim AS frontend-builder
WORKDIR /explorer
COPY explorer/package*.json ./
RUN npm ci --silent
COPY explorer/ ./
RUN npm run build

# ── Stage 2: Build Rust Backend ───────────────────────────────────────────────
# Ubuntu 24.04 is required because ort (ONNX Runtime) 2.0.0-rc.12 prebuilt binaries
# link against glibc 2.38+ symbols (__isoc23_strtoll etc.) not present in Debian 12.
FROM ubuntu:24.04 AS builder

# Install native dependencies for:
# - OpenSSL (reqwest)
# - ZK proofs (arkworks needs build tools)
# - ML/ONNX (math libraries)
# - WASM (wasmtime dependencies)
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    curl \
    pkg-config \
    libssl-dev \
    build-essential \
    clang \
    llvm \
    libclang-dev \
    cmake \
    libopenblas-dev \
    protobuf-compiler \
    libprotobuf-dev \
    git \
    && rm -rf /var/lib/apt/lists/*

# Install Rust toolchain manually (official rust images are Debian-based and too old for glibc)
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain 1.90.0 \
    && . "$HOME/.cargo/env" \
    && cargo --version \
    && rustc --version
ENV PATH="/root/.cargo/bin:$PATH"

WORKDIR /build

# Copy the built frontend into the expected location for rust-embed
COPY --from=frontend-builder /explorer/dist /build/explorer/dist

# Cache dependency layer separately from source.
COPY Cargo.toml ./
COPY Cargo.lock ./

# Core crates
COPY crates/common/Cargo.toml   crates/common/
COPY crates/cli/Cargo.toml      crates/cli/
COPY crates/resolver/Cargo.toml crates/resolver/
COPY crates/validator/Cargo.toml crates/validator/
COPY crates/consensus/Cargo.toml crates/consensus/
COPY crates/node/Cargo.toml     crates/node/

# Phase 1: Advanced Validation crates
COPY crates/zk-validator/Cargo.toml     crates/zk-validator/
COPY crates/ml-validator/Cargo.toml     crates/ml-validator/
COPY crates/wasm-sandbox/Cargo.toml     crates/wasm-sandbox/

# Phase 2: Enterprise crates
COPY crates/threshold-encryption/Cargo.toml crates/threshold-encryption/
COPY crates/cross-chain/Cargo.toml        crates/cross-chain/

# Phase 3: Ecosystem crates
COPY crates/insurance/Cargo.toml        crates/insurance/
COPY crates/db-sync/Cargo.toml          crates/db-sync/
COPY crates/ipfs-pinner/Cargo.toml      crates/ipfs-pinner/

# Stub sources so Cargo can resolve dependencies before we copy real code.
RUN mkdir -p crates/common/src     && echo "pub fn _stub() {}" > crates/common/src/lib.rs
RUN mkdir -p crates/cli/src        && echo "fn main() {}"      > crates/cli/src/main.rs
RUN mkdir -p crates/resolver/src   && echo "pub fn _stub() {}" > crates/resolver/src/lib.rs
RUN mkdir -p crates/validator/src  && echo "pub fn _stub() {}" > crates/validator/src/lib.rs
RUN mkdir -p crates/consensus/src  && echo "pub fn _stub() {}" > crates/consensus/src/lib.rs
RUN mkdir -p crates/node/src       && echo "fn main() {}"      > crates/node/src/main.rs

# Phase 1 stubs
RUN mkdir -p crates/zk-validator/src     && echo "pub fn _stub() {}" > crates/zk-validator/src/lib.rs
RUN mkdir -p crates/ml-validator/src     && echo "pub fn _stub() {}" > crates/ml-validator/src/lib.rs
RUN mkdir -p crates/wasm-sandbox/src     && echo "pub fn _stub() {}" > crates/wasm-sandbox/src/lib.rs

# Phase 2 stubs
RUN mkdir -p crates/threshold-encryption/src && echo "pub fn _stub() {}" > crates/threshold-encryption/src/lib.rs
RUN mkdir -p crates/cross-chain/src        && echo "pub fn _stub() {}" > crates/cross-chain/src/lib.rs

# Phase 3 stubs
RUN mkdir -p crates/insurance/src        && echo "pub fn _stub() {}" > crates/insurance/src/lib.rs
RUN mkdir -p crates/db-sync/src          && echo "pub fn _stub() {}" > crates/db-sync/src/lib.rs
RUN mkdir -p crates/ipfs-pinner/src      && echo "pub fn _stub() {}" > crates/ipfs-pinner/src/lib.rs

# Faucet stub
RUN mkdir -p crates/faucet/src           && echo "fn main() {}"      > crates/faucet/src/main.rs

# Pre-build dependencies (cached layer)
RUN cargo build --release --package chain-registry-node 2>/dev/null || true

# Copy real source and build properly.
COPY crates/ crates/
COPY circuits/ circuits/
COPY contracts/ contracts/
COPY docker-compose*.yml ./
COPY testnet/ testnet/

# Touch files to ensure rebuild
RUN touch crates/*/src/*.rs

# Build the node with all features
RUN cargo build --release --package chain-registry-node

# Build CLI tools (includes TUI explorer)
RUN cargo build --release --package chain-registry-cli

# Build Faucet service (testnet)
RUN cargo build --release --package faucet

# ── Stage 3: Runtime ──────────────────────────────────────────────────────────
# Must match builder glibc version (Ubuntu 24.04 has glibc 2.39)
FROM ubuntu:24.04 AS runtime

# Production-hardened validator deps:
# - ca-certificates & curl: for IPFS / Eth RPC communication
# - libssl3: for crypto operations
# - libopenblas0: for ML math operations
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    curl \
    libssl3 \
    libopenblas0 \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy binaries
COPY --from=builder /build/target/release/creg-node /app/creg-node
COPY --from=builder /build/target/release/creg /app/creg
COPY --from=builder /build/target/release/faucet /app/faucet

# Copy WASM validators (for wasm-sandbox)
COPY --from=builder /build/crates/cli/validators /app/validators

# Copy circuits (for ZK validation)
COPY --from=builder /build/circuits /app/circuits

# Non-root user for security.
RUN useradd -r -s /bin/false creg
RUN mkdir -p /data && chown creg:creg /data
USER creg

VOLUME ["/data"]
EXPOSE 8080 50051

HEALTHCHECK --interval=20s --timeout=10s --retries=3 \
  CMD curl -f http://localhost:8080/v1/health || exit 1

ENTRYPOINT ["/app/creg-node"]
