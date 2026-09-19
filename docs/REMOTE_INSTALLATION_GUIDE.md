# Chain Registry (CREG) — Remote Installation & Distribution Guide

> **Document Version:** 1.0.0  
> **Target Network:** CREG Sepolia Decentralized Testnet  
> **CDN Endpoint:** `https://spec.testnet.cregnet.dev`  
> **Last Verified:** September 2026

---

## 1. Executive Summary & Root Cause Analysis

### 1.1 The Question: Why is Remote Installation Different?
When you develop CREG on your local workstation, installing the CLI and node feels effortless because your workstation already contains:
1. **The Full Source Tree:** The local Git repository (`chain-registry-blockchain-CREG-`) with all 20+ crates.
2. **The Rust Toolchain:** `rustc`, `cargo`, and LLVM.
3. **Native C/C++ Build Dependencies:** `protobuf-compiler` (`protoc`), `clang`, `cmake`, OpenSSL headers, and Visual Studio C++ / GCC toolchains.
4. **Precompiled Build Caches:** Existing objects in `target/debug` or `target/release`.

When an external developer, node operator, or user attempts to install CREG on a **remote machine** (a cloud VM, a developer laptop, or a bare-metal server):
- **They do NOT have the source code cloned.**
- **They do NOT have the Rust toolchain or C++ build tools installed.** Compiling from source on a clean machine requires downloading over 200 crates, consuming gigabytes of RAM, and taking 15–30 minutes, often failing if native tools like `protoc` or `libclang` are absent.
- **They expect zero-friction distribution:** Standard modern developer tools (`docker`, `kubectl`, `foundry`, `rustup`) install via a single curl/powershell command or precompiled binary archives that run instantly.

### 1.2 Does It Already Exist? (Status & What Was Improved)
- **What Existed Previously:**
  Scripts named `scripts/install-creg.sh` and `install-creg.ps1` were in the repo, but they were designed primarily for local development: if no `--version` was passed, they attempted to run `cargo build` in a relative path. If executed via piped curl (`curl | bash`), they failed immediately because `$0` is not a file on disk and `cargo` was missing on remote systems.
- **What Has Been Done & Is Live Now:**
  1. **Public CDN Hosting:** The CREG Testnet Spec Server (`https://spec.testnet.cregnet.dev`) now actively hosts both universal install scripts (`/install.sh` and `/install.ps1`) and precompiled release archives under `/dist/`.
  2. **Automated Platform Detection:** The scripts automatically detect the user's OS and CPU architecture (Linux x86_64, macOS, Windows amd64).
  3. **Cryptographic SHA-256 Verification:** Installers automatically fetch `SHA256SUMS` and verify the cryptographic checksum of downloaded binaries before unpacking.
  4. **Auto-Configuration of Testnet Spec & PATH:** The scripts automatically download the signed testnet chain spec (`chain-spec.sepolia.json` and `.sig`) to `~/.creg/` and register `~/.creg/bin` into the user's shell profile (`.bashrc`/`.zshrc`) or Windows Registry `PATH`.
  5. **Docker Fallback Shim:** If an unsupported platform or environment is detected, the installer offers an instant Docker-based CLI shim with zero host compilation.

---

## 2. Remote Installation Methods

```
                       ┌──────────────────────────────────────────────┐
                       │  Remote User Machine (No Source Repo Needed) │
                       └──────────────────────┬───────────────────────┘
                                              │
                    ┌─────────────────────────┴─────────────────────────┐
                    ▼                                                   ▼
         [ Linux / macOS ]                                         [ Windows ]
   curl -fsSL https://.../install.sh | bash                 irm https://.../install.ps1 | iex
                    │                                                   │
                    ▼                                                   ▼
       Downloads Precompiled Tarball                       Downloads Precompiled Zip
         from CREG Testnet CDN                               from CREG Testnet CDN
                    │                                                   │
                    ▼                                                   ▼
      Verifies SHA-256 Checksum                           Verifies SHA-256 Checksum
                    │                                                   │
                    ▼                                                   ▼
       Extracts to ~/.creg/bin/                            Extracts to %USERPROFILE%\.creg\bin
      Updates PATH in .bashrc/.zshrc                       Updates Windows Registry User PATH
                    │                                                   │
                    └─────────────────────────┬─────────────────────────┘
                                              ▼
                                Ready to use: `creg --help`
```

---

### Method 1: One-Line Remote Installer for Linux & macOS (Recommended)

Run the following command in any Linux or macOS terminal:

```bash
curl -fsSL https://spec.testnet.cregnet.dev/install.sh | bash
```

#### What this command does:
1. Detects OS (`Linux` / `Darwin`) and Architecture (`x86_64` / `arm64`).
2. Downloads `chain-registry-v0.4.4-testnet-linux-amd64.tar.gz` directly from `https://spec.testnet.cregnet.dev/dist/`.
3. Downloads `SHA256SUMS` and verifies that the archive matches the exact cryptographic hash.
4. Extracts `creg` (CLI) and `creg-node` (Daemon) into `~/.creg/bin`.
5. Downloads the active signed Sepolia testnet chain spec (`chain-spec.sepolia.json` and `.sig`) to `~/.creg/`.
6. Appends `export PATH="$HOME/.creg/bin:$PATH"` to your `~/.bashrc` or `~/.zshrc`.

#### Customization Flags:
You can pass custom options to the installer:
```bash
# Install a specific release version
curl -fsSL https://spec.testnet.cregnet.dev/install.sh | bash -s -- --version v0.4.4-testnet

# Install to a custom directory (e.g. /usr/local/bin)
curl -fsSL https://spec.testnet.cregnet.dev/install.sh | bash -s -- --install-dir /usr/local/bin

# Install the Docker-based CLI shim instead of native binaries
curl -fsSL https://spec.testnet.cregnet.dev/install.sh | bash -s -- --docker
```

---

### Method 2: One-Line Remote Installer for Windows (Recommended)

Open **PowerShell** (no Administrator privileges required) and run:

```powershell
irm https://spec.testnet.cregnet.dev/install.ps1 | iex
```

#### What this command does:
1. Downloads `chain-registry-v0.4.4-testnet-windows-amd64.zip` from `https://spec.testnet.cregnet.dev/dist/`.
2. Downloads `SHA256SUMS` and verifies the SHA-256 hash using `Get-FileHash`.
3. Extracts `creg.exe` to `$env:USERPROFILE\.creg\bin\creg.exe`.
4. Syncs the signed testnet specification to `$env:USERPROFILE\.creg\chain-spec.sepolia.json`.
5. Permanently registers `$env:USERPROFILE\.creg\bin` into the Windows User Environment `Path` registry so all future PowerShell and CMD terminals can run `creg` globally.
6. Refreshes the current session PATH so you can use `creg` immediately.

---

### Method 3: Docker-Native Execution (Zero Installation)

For operators or CI/CD pipelines that prefer container isolation with zero host installation:

```bash
# Pull the prebuilt testnet image
docker pull ghcr.io/samuel-1-avson/chain-registry:latest

# Run any CLI command interactively
docker run --rm -it \
  -v "$HOME/.creg:/root/.creg" \
  -v "$(pwd):/work" \
  -w /work \
  ghcr.io/samuel-1-avson/chain-registry:latest creg --help
```

To create a permanent local alias:
```bash
alias creg='docker run --rm -it -v "$HOME/.creg:/root/.creg" -v "$(pwd):/work" -w /work ghcr.io/samuel-1-avson/chain-registry:latest creg'
```

---

### Method 4: Manual Binary Download & Verification

If your remote environment has restricted egress or strict compliance policies, you can download the release artifacts directly:

| Platform | Archive Asset | SHA-256 Hash |
| :--- | :--- | :--- |
| **Linux (x86_64)** | [`chain-registry-v0.4.4-testnet-linux-amd64.tar.gz`](https://spec.testnet.cregnet.dev/dist/chain-registry-v0.4.4-testnet-linux-amd64.tar.gz) | `5e84bafc338829fc392b466f0a689fda56ac2c262bc12ca8e8375e8a98b3e814` |
| **Windows (x86_64)** | [`chain-registry-v0.4.4-testnet-windows-amd64.zip`](https://spec.testnet.cregnet.dev/dist/chain-registry-v0.4.4-testnet-windows-amd64.zip) | `1ae2c64554cb71ae2960da3facf62e5cb747dab4f1aef2d5f708c08ec4516281` |
| **Checksums** | [`SHA256SUMS`](https://spec.testnet.cregnet.dev/dist/SHA256SUMS) | Signed verification manifest |

#### Manual Linux Steps:
```bash
# 1. Download archive and checksums
curl -fsSL https://spec.testnet.cregnet.dev/dist/chain-registry-v0.4.4-testnet-linux-amd64.tar.gz -o creg.tar.gz
curl -fsSL https://spec.testnet.cregnet.dev/dist/SHA256SUMS -o SHA256SUMS

# 2. Verify SHA-256
sha256sum creg.tar.gz
grep "chain-registry-v0.4.4-testnet-linux-amd64.tar.gz" SHA256SUMS

# 3. Extract to /usr/local/bin or ~/.creg/bin
mkdir -p ~/.creg/bin
tar -xzf creg.tar.gz -C ~/.creg/bin
chmod +x ~/.creg/bin/creg ~/.creg/bin/creg-node
```

---

### Method 5: Compiling From Source (For Rust Developers)

If a remote user has Rust installed and specifically desires to build from source:

```bash
# 1. Install Rust if not present
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y

# 2. Clone repository
git clone https://github.com/samuel-1-avson/chain-registry-blockchain-CREG-.git
cd chain-registry-blockchain-CREG-/chain-registry

# 3. Compile and install using the build script
./scripts/install-creg.sh --build
```

---

## 3. Post-Installation Verification

Once installed, verify that the CLI is communicating with the decentralized testnet:

### 3.1 Check Version
```bash
creg --version
# Output: creg 0.1.0
```

### 3.2 Verify Testnet Node Connectivity
```bash
creg testnet status
```

### 3.3 Connect Your Wallet (Dual-Key Architecture)
CREG features a unified Dual-Key Architecture (DKA) that binds your Ethereum L1 wallet with high-speed Ed25519 consensus signing without manual key juggling:

```bash
# Interactive MetaMask 1-Click Login (Opens browser bridge):
creg login

# Check active authenticated profile:
creg profile

# Or headless / CI login with private key:
creg login --private-key 0xYourEthPrivateKey
```

*(Legacy manual key generation via `creg keygen publisher` remains fully supported).*

### 3.4 Request Testnet Tokens from Faucet
```bash
creg testnet drip --address 0xYourEthAddress
```

---

## 4. Running a Remote Validator Node

For users setting up an **independent remote validator** system:

```bash
# Download and execute the automated validator onboarding script:
curl -fsSL https://raw.githubusercontent.com/samuel-1-avson/chain-registry-blockchain-CREG-/main/chain-registry/testnet/start-external-validator.sh | bash
```

The script will:
1. Automatically provision an isolated data directory at `~/.creg-validator`.
2. Generate consensus keys (`validator.key`) and an operator API key.
3. Fetch the signed `chain-spec.sepolia.json` from the spec server.
4. Launch the validator container connected to the live bootnodes (`35.225.225.20:9000`).

---

## 5. Summary Reference Card

| Action | Linux / macOS Command | Windows PowerShell Command |
| :--- | :--- | :--- |
| **Install CLI & Node** | `curl -fsSL https://spec.testnet.cregnet.dev/install.sh \| bash` | `irm https://spec.testnet.cregnet.dev/install.ps1 \| iex` |
| **Verify Installation** | `creg --version` | `creg --version` |
| **Check Network** | `creg blocks` | `creg blocks` |
| **Create Key** | `creg keygen publisher` | `creg keygen publisher` |
| **Run Validator** | `bash <(curl -fsSL https://spec.testnet.cregnet.dev/start-external-validator.sh)` | `.\testnet\start-external-validator.ps1` |
