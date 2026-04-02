# Docker Deployment Guide

**Chain Registry v0.2.0** - Complete System with Phases 1-3

This guide covers deploying the full Chain Registry system with all advanced features using Docker Compose.

---

## Prerequisites

- Docker 20.10+ and Docker Compose 2.0+
- 8GB RAM minimum (16GB recommended)
- 50GB free disk space
- Linux/macOS/Windows with WSL2

---

## Quick Start

### 1. Clone and Navigate

```bash
cd chain-registry/chain-registry
```

### 2. Configure Environment

```bash
# Copy example environment file
cp .env.example .env

# Edit with your settings
nano .env
```

### 3. Generate Validator Keys

```bash
# Run key generation (if creg binary is available locally)
# Or use Docker:
docker run --rm -v $(pwd)/keys:/keys chain-registry/chain-registry:latest keygen validator --output /keys/node1.key
```

### 4. Deploy the System

```bash
# Build and start all services
docker-compose up -d

# Or with explicit build
docker-compose up -d --build
```

### 5. Verify Deployment

```bash
# Check all services are running
docker-compose ps

# Check logs
docker-compose logs -f node-1

# Test health endpoint
curl http://localhost:8080/v1/health
```

---

## Services Overview

| Service | Port | Description |
|---------|------|-------------|
| `node-1` | 8080, 4001 | Primary validator with all features |
| `node-2` | 8082 | Secondary validator |
| `node-3` | 8083 | Third validator |
| `ipfs` | 5001, 8081 | IPFS node for package storage |
| `anvil` | 8545 | Local Ethereum testnet |
| `deploy-contracts` | - | Contract deployment service |

---

## Phase 1 Features (ZK/ML/WASM)

### ZK Validation

Enabled by default. To verify:

```bash
# Check ZK circuits are mounted
docker exec creg-node-1 ls /app/circuits

# Submit package with ZK proof
docker-compose run --rm cli advanced zk-proof ./package.tgz
```

### ML Threat Detection

The ML validator runs automatically on all submissions. Check logs:

```bash
docker-compose logs node-1 | grep "ml_validator"
```

### WASM Sandbox

WASM validators are mounted at `/app/validators`. To add custom validators:

```bash
# Place .wasm files in ./validators directory
mkdir -p validators
cp my-validator.wasm validators/

# Restart nodes
docker-compose restart node-1 node-2 node-3
```

---

## Phase 2 Features (Enterprise)

### Private Registry

Configure organization creation:

```bash
# Use CLI to create private organization
docker-compose run --rm cli private create-org \
  --name "MyOrg" \
  --threshold 3 \
  --validators validator1.pub,validator2.pub,validator3.pub
```

### Cross-Chain

Cross-chain features are configured but require external bridge setup for production.

---

## Phase 3 Features (Ecosystem)

### Token Operations

```bash
# Check token balance
docker-compose run --rm cli token balance --address 0x...

# Stake tokens
docker-compose run --rm cli stake --amount 1000 --role validator
```

### Insurance

```bash
# Purchase insurance for package
docker-compose run --rm cli insurance purchase \
  --package "npm:express@4.18.2" \
  --coverage 10.0

# Check insurance pool health
curl http://localhost:8080/v1/insurance/health
```

### Governance

```bash
# Create proposal
docker-compose run --rm cli governance propose \
  --target 0x... \
  --calldata 0x... \
  --description "Update staking rewards"

# Cast vote
docker-compose run --rm cli governance vote --proposal 1 --support 1
```

---

## Configuration Options

### Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `CREG_ZK_ENABLED` | `true` | Enable ZK proof validation |
| `CREG_ML_ENABLED` | `true` | Enable ML threat detection |
| `CREG_WASM_ENABLED` | `true` | Enable WASM sandboxing |
| `CREG_INSURANCE_ENABLED` | `true` | Enable insurance features |
| `CREG_WASM_MEMORY_LIMIT` | `268435456` | WASM memory limit (bytes) |
| `CREG_WASM_TIMEOUT` | `30` | WASM execution timeout (seconds) |

### Volume Mounts

| Host Path | Container Path | Purpose |
|-----------|---------------|---------|
| `./circuits` | `/app/circuits` | ZK circuits for validation |
| `./validators` | `/app/validators` | WASM validator binaries |
| `./models` | `/app/models` | ML model files (optional) |

---

## Monitoring

### Health Checks

```bash
# Node health
curl http://localhost:8080/v1/health

# IPFS health
curl http://localhost:5001/api/v0/id

# Anvil health
cast chain-id --rpc-url http://localhost:8545
```

### Logs

```bash
# All services
docker-compose logs -f

# Specific service
docker-compose logs -f node-1

# Filter for ZK validation
docker-compose logs node-1 | grep "zk_validator"

# Filter for ML detection
docker-compose logs node-1 | grep "ml_validator"
```

### Metrics

Prometheus metrics available at: `http://localhost:8080/metrics`

---

## Troubleshooting

### Container Won't Start

```bash
# Check logs
docker-compose logs node-1

# Rebuild
docker-compose up -d --build --force-recreate node-1
```

### Contract Deployment Fails

```bash
# Check Anvil is running
docker-compose ps anvil

# Restart deployment
docker-compose restart deploy-contracts
```

### ZK Validation Errors

```bash
# Verify circuits are present
docker exec creg-node-1 ls -la /app/circuits/

# Check circuit compilation
ls -la circuits/
```

### Out of Memory

```bash
# Increase Docker memory limit
# Docker Desktop > Settings > Resources > Memory: 16GB

# Or use swap
docker-compose up -d --compatibility
```

---

## Production Deployment

### Security Hardening

1. **Use External Ethereum Node**
   ```yaml
   CREG_ETH_RPC: https://mainnet.infura.io/v3/YOUR_KEY
   ```

2. **Enable TLS**
   ```yaml
   CREG_TLS_CERT: /path/to/cert.pem
   CREG_TLS_KEY: /path/to/key.pem
   ```

3. **Set Resource Limits**
   ```yaml
   deploy:
     resources:
       limits:
         cpus: '2'
         memory: 4G
   ```

### Scaling

```bash
# Scale validators horizontally
docker-compose up -d --scale node=5

# Use external IPFS cluster
docker-compose -f docker-compose.yml -f docker-compose.ipfs-cluster.yml up -d
```

---

## Cleanup

```bash
# Stop all services
docker-compose down

# Remove volumes (WARNING: deletes data)
docker-compose down -v

# Remove images
docker-compose down --rmi all
```

---

## Support

- **Documentation**: See `FINAL_SYSTEM_ANALYSIS_REPORT.md`
- **Issues**: Check GitHub Issues
- **Logs**: `docker-compose logs > debug.log`

---

**System Version**: v0.2.0 (Phases 1-3 Complete)  
**Last Updated**: March 30, 2026
