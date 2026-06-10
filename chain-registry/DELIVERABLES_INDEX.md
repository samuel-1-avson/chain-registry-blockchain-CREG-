# Deliverables index

> **Updated:** 2026-06-09  
> Map of active documentation and operator surfaces.

## Documentation entrypoints

| File | Role |
| ---- | ---- |
| [`../README.md`](../README.md) | Project overview and live service URLs |
| [`../docs/README.md`](../docs/README.md) | Central documentation index |
| [`DEEP_DIVE_ANALYSIS.md`](DEEP_DIVE_ANALYSIS.md) | Architecture, subsystems, issue registry |
| [`TESTNET_READINESS_REPORT.md`](TESTNET_READINESS_REPORT.md) | Testnet readiness snapshot |
| [`../docs/NEXT_WORK.md`](../docs/NEXT_WORK.md) | Prioritized open work |
| [`../docs/REMEDIATION_BACKLOG.md`](../docs/REMEDIATION_BACKLOG.md) | Live remediation status |
| [`../docs/GCP-BUDGET-ARCHITECTURE.md`](../docs/GCP-BUDGET-ARCHITECTURE.md) | VM + Firebase cost model |
| [`../docs/WAITLIST_FIREBASE_DEPLOY.md`](../docs/WAITLIST_FIREBASE_DEPLOY.md) | Waitlist Firebase deploy |

## Testnet & public hosting

| File | Role |
| ---- | ---- |
| [`testnet/OPERATOR.md`](testnet/OPERATOR.md) | 3-node Sepolia operator runbook |
| [`testnet/gcp-public-hosting.md`](testnet/gcp-public-hosting.md) | GCP + Cloudflare HTTPS (HOSTING-301) |
| [`testnet/hosting-301-verify.ps1`](testnet/hosting-301-verify.ps1) | Verify public vhosts |
| [`testnet/gcp/deploy-stack.ps1`](testnet/gcp/deploy-stack.ps1) | Sync repo + start stack on VM |
| [`testnet/gcp/deploy-waitlist.ps1`](testnet/gcp/deploy-waitlist.ps1) | Build + deploy waitlist static site |
| [`testnet/gcp/deploy-waitlist-firebase.ps1`](testnet/gcp/deploy-waitlist-firebase.ps1) | Deploy Firestore rules + `registerWaitlist` |
| [`testnet/docker-compose.3node.yml`](testnet/docker-compose.3node.yml) | Core 3-node compose |
| [`testnet/docker-compose.3node-services.yml`](testnet/docker-compose.3node-services.yml) | Explorer, faucet, spec, IPFS |
| [`testnet/docker-compose.3node-ingress.yml`](testnet/docker-compose.3node-ingress.yml) | Caddy TLS ingress |
| [`testnet/docker-compose.waitlist.yml`](testnet/docker-compose.waitlist.yml) | Waitlist nginx on VM |
| [`testnet/SOAK_TEST.md`](testnet/SOAK_TEST.md) | Soak scenario catalog |
| [`testnet/README.md`](testnet/README.md) | Testnet directory overview |

## Waitlist app

| Path | Role |
| ---- | ---- |
| [`../Creg-waitlist/`](../Creg-waitlist/) | Vite SPA + Firebase functions |
| [`../Creg-waitlist/functions/`](../Creg-waitlist/functions/) | `registerWaitlist` callable |
| [`testnet/waitlist/`](testnet/waitlist/) | Production static `dist/` + nginx Dockerfile |

## Local bootstrap

| File | Purpose |
| ---- | ------- |
| [`local-testnet.ps1`](local-testnet.ps1) | Local three-validator bootstrap |
| [`DOCKER.md`](DOCKER.md) | Docker compose profiles |
| [`docker-compose.yml`](docker-compose.yml) | Single-node Sepolia dev stack |

## Deployment surfaces

| File | Purpose |
| ---- | ------- |
| [`testnet/bootstrap/README.md`](testnet/bootstrap/README.md) | Bootstrap node deployment |
| [`testnet/spec-server/README.md`](testnet/spec-server/README.md) | Signed chain-spec hosting |
| [`k8s/`](k8s/) | Kubernetes manifests |

## Component READMEs

| File | Purpose |
| ---- | ------- |
| [`contracts/README.md`](contracts/README.md) | Solidity contracts |
| [`migrations/README.md`](migrations/README.md) | Database migrations |
| [`observability/README.md`](observability/README.md) | Prometheus/Grafana |
| [`config/sandbox/rootfs/README.md`](config/sandbox/rootfs/README.md) | nsjail sandbox rootfs |
| [`../circuits/README.md`](../circuits/README.md) | ZK circuits |
