# Chain Registry — Quick Start Guide

> Goal: get a node running quickly and verify health/status surfaces.

**Scale testnet (10k+ validators):** [TESTNET_SCALE_ARCHITECTURE.md](../../docs/TESTNET_SCALE_ARCHITECTURE.md)  
**External validators:** [VALIDATOR_ONBOARDING_CHECKLIST.md](../../docs/VALIDATOR_ONBOARDING_CHECKLIST.md)  
**Publishers/developers:** [PUBLIC_TESTNET_QUICKSTART.md](../../docs/PUBLIC_TESTNET_QUICKSTART.md)

## Choose your path

| Path | Time | What you get | Best for |
|------|------|--------------|----------|
| A. Single node (native) | 15 min | One node on your machine | Local development |
| B. Bootstrap host (Docker) | 15–20 min | Shared services + `node-1` | Local bootstrap |
| C. External validator | 1–2 hours | Join public Sepolia testnet | Genesis Alpha cohort |


## Path A: Single node — native

```bash
git clone https://github.com/samuel-1-avson/chain-registry-blockchain-CREG-.git
cd chain-registry/testnet
cp .env.example .env
```

Linux/macOS: `./start-local-node.sh -n my-node`  
Windows: `.\start-local-node.ps1 -NodeId my-node`

```bash
curl http://localhost:8080/v1/health
curl http://localhost:8080/v1/chain/stats | jq
```

## Path B: Bootstrap host — Docker

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass -File testnet.ps1
```

Or:

```bash
docker compose --env-file .env.testnet -f docker-compose.testnet.yml up -d --build
```

## Path C: External validator (public testnet)

Follow the full checklist — do **not** use the retired 3-node Sepolia lab.

1. [VALIDATOR_ONBOARDING_CHECKLIST.md](../../docs/VALIDATOR_ONBOARDING_CHECKLIST.md)
2. [GENESIS_ALPHA_COHORT_PLAYBOOK.md](../../docs/GENESIS_ALPHA_COHORT_PLAYBOOK.md) (if invited)
3. Deploy with `docker-compose.validator.yml` or release `creg-node`
4. Point at `https://api.testnet.cregnet.dev` for registration smoke tests

## Supported smoke checks

```bash
curl http://localhost:8080/v1/health
curl http://localhost:8080/v1/chain/stats | jq '{height: .block_height, peers: .peer_count, validators: .validator_count}'
creg testnet status --node-url http://localhost:8080
```

## Publishing a package

```bash
creg --node-url http://localhost:8080 publish ./my-package.tgz \
  --key-file ~/.creg/publisher.key \
  --publisher-address 0xYourPublisherAddress
```

## Next steps

1. Developers: Path A or B, then [PUBLIC_TESTNET_QUICKSTART.md](../../docs/PUBLIC_TESTNET_QUICKSTART.md)
2. Validators: Path C when cohort invites open
3. Maintainers: [OPERATOR.md](./OPERATOR.md) and [gcp/README.md](./gcp/README.md)