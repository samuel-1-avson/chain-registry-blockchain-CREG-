# Makefile — chain-registry dev commands

.PHONY: build test lint clean node cluster contracts deploy-local fmt check testnet testnet-smoke testnet-no-explorer testnet-stop testnet-reset testnet-logs

POWERSHELL ?= pwsh
TESTNET_COMPOSE = docker compose --project-directory . --env-file .env.testnet -f docker-compose.testnet.yml

# ── Rust ──────────────────────────────────────────────────────────────────────

build:
	cargo build --release

test:
	cargo test --workspace -- --nocapture

lint:
	cargo clippy --workspace --all-targets -- -D warnings

fmt:
	cargo fmt --all

check:
	cargo check --workspace

clean:
	cargo clean

# ── Node ──────────────────────────────────────────────────────────────────────

## Run a single dev node (no validators, data in ./dev-data)
node:
	CREG_LISTEN=0.0.0.0:8080 \
	CREG_DATA_DIR=./dev-data \
	CREG_NODE_ID=dev-node \
	CREG_IS_VALIDATOR=true \
	CREG_BLOCK_INTERVAL=3 \
	RUST_LOG=info,chain_registry_node=debug \
	cargo run --release --bin creg-node

## Spin up the full 3-node + IPFS + Anvil cluster
cluster:
	docker compose up --build

cluster-down:
	docker compose down -v

## Start the bootstrap testnet (GNU make convenience wrapper around ./testnet.ps1)
testnet:
	$(POWERSHELL) -NoProfile -ExecutionPolicy Bypass -File testnet.ps1

## Start the bootstrap testnet and run the smoke checks at the end
testnet-smoke:
	$(POWERSHELL) -NoProfile -ExecutionPolicy Bypass -File testnet.ps1 -RunSmokeTests

## Start the bootstrap testnet without the standalone web explorer
testnet-no-explorer:
	$(POWERSHELL) -NoProfile -ExecutionPolicy Bypass -File testnet.ps1 -SkipExplorer

## Stop the bootstrap testnet services without deleting volumes
testnet-stop:
	$(TESTNET_COMPOSE) down --remove-orphans

## Stop the bootstrap testnet and delete volumes for a clean reset
testnet-reset:
	$(TESTNET_COMPOSE) down -v --remove-orphans

## Tail bootstrap testnet logs
testnet-logs:
	$(TESTNET_COMPOSE) logs -f

# ── CLI shims ─────────────────────────────────────────────────────────────────

## Install PATH shims so npm/pip/cargo are intercepted
install-shims: build
	./target/release/creg setup-shims
	@echo ""
	@echo "Add this to your shell profile:"
	@echo '  export PATH="$$HOME/.local/bin:$$PATH"'

remove-shims:
	./target/release/creg remove-shims

# ── Smart contracts ───────────────────────────────────────────────────────────

## Install Foundry (one-time)
install-foundry:
	curl -L https://foundry.paradigm.xyz | bash
	foundryup

## Compile contracts
contracts:
	forge build

## Run Solidity tests
test-contracts:
	forge test -vvv

## Run tests with gas report
gas:
	forge test --gas-report

## Deploy to local Anvil
deploy-local:
	@echo "Starting Anvil..."
	anvil &
	sleep 2
	DEPLOYER_KEY=0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80 \
	GENESIS_SIGNERS="" \
	GOVERNANCE_THRESHOLD=1 \
	forge script contracts/script/Deploy.s.sol \
		--rpc-url http://127.0.0.1:8545 \
		--broadcast \
		-vvvv

## Deploy to Sepolia testnet
deploy-sepolia:
	forge script contracts/script/Deploy.s.sol \
		--rpc-url $$SEPOLIA_RPC_URL \
		--broadcast \
		--verify \
		-vvvv

# ── Utilities ─────────────────────────────────────────────────────────────────

## Generate a new Ed25519 publisher keypair
keygen:
	cargo run --bin creg -- keygen

## Check the trust status of a package
status:
	cargo run --bin creg -- status $(PKG) --ecosystem $(ECO)

## Watch logs from all docker compose services
logs:
	docker compose logs -f

## Print chain stats from the local dev node
stats:
	curl -s http://localhost:8080/v1/chain/stats | jq .

## List pending pool
pending:
	curl -s http://localhost:8080/v1/pending | jq .
