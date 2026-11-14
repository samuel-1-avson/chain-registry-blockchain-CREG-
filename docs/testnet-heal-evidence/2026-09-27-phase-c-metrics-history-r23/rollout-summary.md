# R23 O1 metrics history staging

Captured on 2026-09-27. R23 was staged on the R7 candidate observer O1 (`creg-testnet-vm`, `us-central1-b`) only. The four validators and the other three observers were not redeployed.

## Verification

- The node remained a non-voting observer on the signed R7 chain at finalized height 14, with the expected block hash and four active validators.
- `/v1/public/metrics/history?range=5m` returned 20 retained samples from all seven Prometheus series. A direct query to the real Prometheus collector with the exact R7 chain, signed network-genesis identity, and `metrics_source=explorer-primary` label returned the same tip height, 14.
- The local web Metrics page, proxied through an SSH/IAP tunnel to O1, rendered 60 real 30-minute samples across eight chart surfaces. No browser page errors were captured. The screenshot is `metrics-page-r23.png`.
- The R23 CLI TUI displayed `Prometheus history: ready`, `Tip 14`, and 120 retained samples. The raw and cleaned terminal captures are `cli-metrics-r23.ansi` and `cli-metrics-r23.txt`.
- The post-stage readiness capture reports all four validators and four observers ready, recovery queues clear, and PBFT rounds clear.
- R22 remains preserved as a stopped container. R23 uses the existing `/data` bind mount, the existing read-only scanner mount, and both required Docker networks. No chain reset or public cutover occurred.

## Rollout guard note

The first post-start guard reached the API before peer discovery completed and tripped its immediate `peer_count > 0` assertion. It did not roll back or delete either container. After peer reconnection, O1 reported two peers and the delayed API, Prometheus, explorer, CLI, and eight-node readiness checks passed. The saved rollout guard now waits for R7 identity and peer readiness before asserting the remaining post-start conditions.