# Chain Registry Testnet

A complete local testnet environment for developing, testing, and experimenting with Chain Registry without using real tokens or mainnet resources.

## Overview

The CREG Testnet provides:
- **Free Test Tokens (tCREG)**: Unlimited faucet for testing
- **Fast Blocks**: 2-second block time for rapid iteration
- **Relaxed Requirements**: Lower staking minimums, instant unbonding
- **Local Infrastructure**: Self-contained with no external dependencies
- **Full Feature Parity**: All mainnet features available for testing

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                     CREG Testnet Stack                          │
├─────────────────────────────────────────────────────────────────┤
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────────────────┐ │
│  │   Anvil     │  │    IPFS     │  │      PostgreSQL         │ │
│  │  (Ethereum) │  │  (Storage)  │  │      (Database)         │ │
│  │  :8545      │  │   :5001     │  │       :5432             │ │
│  └──────┬──────┘  └──────┬──────┘  └───────────┬─────────────┘ │
│         │                │                     │               │
│         └────────────────┼─────────────────────┘               │
│                          │                                     │
│              ┌───────────▼────────────┐                        │
│              │    CREG Node           │                        │
│              │    (Testnet Mode)      │                        │
│              │    :8080               │                        │
│              └───────────┬────────────┘                        │
│                          │                                     │
│         ┌────────────────┼────────────────┐                   │
│         │                │                │                   │
│  ┌──────▼──────┐  ┌──────▼──────┐  ┌──────▼──────┐           │
│  │   Faucet    │  │  Explorer   │  │   TUI UI    │           │
│  │   :8081     │  │   :3000     │  │  (optional) │           │
│  └─────────────┘  └─────────────┘  └─────────────┘           │
└─────────────────────────────────────────────────────────────────┘
```

## Quick Start

### Prerequisites

- [Docker](https://docs.docker.com/get-docker/) and Docker Compose
- [Foundry](https://getfoundry.sh) (for contract deployment)
- `curl` or similar HTTP client

### 1. Start the Infrastructure

```bash
cd testnet

# Start all services
docker-compose -f docker-compose.testnet.yml up -d

# Wait for services to be ready
sleep 5

# Check status
docker-compose -f docker-compose.testnet.yml ps
```

### 2. Deploy Contracts

```bash
# Deploy test token and staking contracts
./deploy-testnet.sh

# This will output contract addresses and environment variables
```

### 3. Get Test Tokens

```bash
# Via CLI
creg testnet drip 0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266

# Or via web UI: http://localhost:8081

# Or via API
curl -X POST http://localhost:8081/api/drip \
  -H 'Content-Type: application/json' \
  -d '{"address":"0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"}'
```

### 4. Stake and Participate

```bash
# Load environment variables from deployment
source testnet/artifacts/testnet.env

# Stake as publisher (minimum 0.001 tCREG)
creg testnet stake-publisher 0.01 --key 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80

# Stake as validator (minimum 0.1 tCREG)
creg testnet stake-validator 0.1 --key 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80
```

### 5. Start Using the Network

```bash
# Check status
creg testnet status

# Open explorer
open http://localhost:3000

# Generate keys for publishing/validating
creg keygen publisher
creg keygen validator

# Publish a package (once staked as publisher)
creg publish ./my-package.tar.gz --key ~/.creg/publisher.key

# Start validator node
export CREG_IS_VALIDATOR=true
export CREG_VALIDATOR_KEY=<64-char-hex-from-keygen>
creg-node
```

## Testnet vs Mainnet Differences

| Feature | Testnet | Mainnet |
|---------|---------|---------|
| **Token** | tCREG (no value) | CREG (real value) |
| **Publisher Stake** | 0.001 tCREG | 0.1 CREG |
| **Validator Stake** | 0.1 tCREG | 10 CREG |
| **Block Time** | 2 seconds | ~12 seconds |
| **Unbonding Period** | 5 minutes | 7 days |
| **Faucet** | Unlimited | N/A |
| **Network** | Local Anvil | Ethereum mainnet |

## Service URLs

| Service | URL | Description |
|---------|-----|-------------|
| Node API | http://localhost:8080 | Chain Registry REST API |
| Faucet | http://localhost:8081 | Test token distribution |
| Explorer | http://localhost:3000 | Web block explorer |
| Ethereum RPC | http://localhost:8545 | Anvil JSON-RPC |
| IPFS API | http://localhost:5001 | IPFS API endpoint |

## CLI Commands

### Testnet-specific Commands

```bash
# Check testnet status
creg testnet status

# Request test tokens from faucet
creg testnet drip <ethereum-address>

# Stake as publisher
creg testnet stake-publisher <amount-eth> --key <private-key>

# Stake as validator
creg testnet stake-validator <amount-eth> --key <private-key>

# Show testnet documentation
creg testnet docs

# Show reset instructions
creg testnet reset
```

### Using with Regular Commands

All regular CLI commands work with testnet when you set the node URL:

```bash
# Use testnet node
export CREG_NODE_URL=http://localhost:8080

# Now all commands use testnet
creg status my-package
creg watch
creg blocks
```

## Pre-funded Accounts

Anvil comes with 20 pre-funded accounts. Here are the first few:

| Account | Address | Private Key | Initial Balance |
|---------|---------|-------------|-----------------|
| Deployer | `0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266` | `0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80` | 10,000 ETH |
| Faucet | `0x70997970C51812dc3A010C7d01b50e0d17dc79C8` | `0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d` | 10,000 ETH + 1M tCREG |

## Managing the Testnet

### View Logs

```bash
# All services
docker-compose -f docker-compose.testnet.yml logs -f

# Specific service
docker-compose -f docker-compose.testnet.yml logs -f creg-node
docker-compose -f docker-compose.testnet.yml logs -f faucet
```

### Reset Testnet

To completely reset the testnet (clear all data):

```bash
# Stop and remove containers
docker-compose -f docker-compose.testnet.yml down -v

# Remove contract artifacts
rm -rf testnet/artifacts

# Start fresh
docker-compose -f docker-compose.testnet.yml up -d
./deploy-testnet.sh
```

### Update Services

```bash
# Rebuild containers after code changes
docker-compose -f docker-compose.testnet.yml build --no-cache

# Restart with new build
docker-compose -f docker-compose.testnet.yml up -d
```

## Troubleshooting

### Faucet returns "Contract not found"

Contracts haven't been deployed. Run:
```bash
./deploy-testnet.sh
```

### "Insufficient balance" when staking

Get test tokens first:
```bash
creg testnet drip <your-address>
```

### Node won't start

Check logs:
```bash
docker-compose -f docker-compose.testnet.yml logs creg-node
```

Common issues:
- `CREG_VALIDATOR_KEY` not set for validator nodes
- Database connection failed (check postgres is healthy)
- Contract addresses not set

### Cannot connect to services

Verify services are running:
```bash
docker-compose -f docker-compose.testnet.yml ps
```

Check ports aren't in use:
```bash
lsof -i :8080  # Node
lsof -i :8081  # Faucet
lsof -i :8545  # Anvil
lsof -i :3000  # Explorer
```

## Development Workflow

### Testing Smart Contracts

```bash
# After deploying, test staking
export TESTNET_TOKEN_ADDR=<token-address>
export TESTNET_STAKING_ADDR=<staking-address>

# Check balance
cast call $TESTNET_TOKEN_ADDR "balanceOf(address)" 0xYourAddress \
  --rpc-url http://localhost:8545

# Test stake
cast send $TESTNET_STAKING_ADDR "stakeAsPublisher(uint256)" 1000000000000000 \
  --private-key 0xYourKey \
  --rpc-url http://localhost:8545
```

### Testing Package Publishing

```bash
# 1. Get tokens
creg testnet drip 0xYourAddress

# 2. Stake as publisher
creg testnet stake-publisher 0.01 --key 0xYourKey

# 3. Generate publisher key
creg keygen publisher

# 4. Create test package
tar czf test-package.tar.gz ./my-project

# 5. Publish
creg publish test-package.tar.gz --key ~/.creg/publisher.key
```

### Testing Validator Setup

```bash
# 1. Get tokens
creg testnet drip 0xYourAddress

# 2. Stake as validator
creg testnet stake-validator 0.1 --key 0xYourKey

# 3. Generate validator key
creg keygen validator

# 4. Get validator private key
export VALIDATOR_KEY=$(cat ~/.creg/validator.key)

# 5. Start validator
export CREG_IS_VALIDATOR=true
export CREG_VALIDATOR_KEY=$VALIDATOR_KEY
export CREG_NODE_URL=http://localhost:8080

# Run locally (outside Docker)
creg-node

# Or use Docker profile
docker-compose -f docker-compose.testnet.yml --profile validator up -d
```

## Advanced Configuration

### Custom Chain ID

Edit `docker-compose.testnet.yml` and set `--chain-id` in the anvil service.

### Custom Block Time

Change `--block-time 2` to desired seconds in anvil service.

### Persist State Across Restarts

Anvil state is already persisted to `anvil-data` volume. To reset:
```bash
docker-compose -f docker-compose.testnet.yml down -v
```

### Fork Mainnet for Testing

```bash
# In docker-compose.testnet.yml, set ANVIL_FORK_URL
environment:
  ANVIL_FORK_URL: https://eth-mainnet.g.alchemy.com/v2/YOUR_KEY
```

## API Reference

### Faucet API

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/` | GET | Web UI |
| `/health` | GET | Health check |
| `/api/stats` | GET | Faucet statistics |
| `/api/drip` | POST | Request tokens |
| `/api/balance/:address` | GET | Get token balance |

### Example: Request Tokens

```bash
curl -X POST http://localhost:8081/api/drip \
  -H 'Content-Type: application/json' \
  -d '{
    "address": "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"
  }'
```

Response:
```json
{
  "success": true,
  "message": "Tokens sent successfully!",
  "tx_hash": "0x...",
  "amount": "1000"
}
```

## Contributing

When adding features to the testnet:

1. Ensure contracts are backwards compatible or version them
2. Update deployment scripts for new contracts
3. Add CLI commands for new testnet features
4. Update this documentation

## Security Notes

⚠️ **Testnet tokens have no value.** Do not attempt to sell or transfer them for value.

⚠️ **Private keys in this repo are public.** Never use them on mainnet or for real funds.

⚠️ **Testnet is ephemeral.** Data may be reset at any time during development.

## Resources

- [Foundry Book](https://book.getfoundry.sh/)
- [Anvil Documentation](https://book.getfoundry.sh/anvil/)
- [Chain Registry Main Documentation](../CHAIN_REGISTRY_GUIDE.md)
