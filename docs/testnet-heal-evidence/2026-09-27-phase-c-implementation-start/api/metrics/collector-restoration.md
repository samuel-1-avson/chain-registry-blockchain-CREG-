# R7 metrics collector restoration

**Captured:** 2026-09-27 09:19 UTC  
**Scope:** candidate observability only. No chain reset or public cutover.

## Live collector evidence

- Host: `creg-testnet-vm` in `us-central1-b`.
- Prometheus container: `creg-gcp-prometheus`, running since `2026-09-27T09:03:12.389012042Z`.
- Health endpoint: `/-/healthy` returned `Prometheus Server is Healthy.`
- Existing TSDB volume: `monitoring_prometheus-gcp-data`, mounted at `/prometheus` from `/var/lib/docker/volumes/monitoring_prometheus-gcp-data/_data`; usage was 8.0 MiB at capture. The named volume was reused; it was not removed, recreated, or cleared.
- Scrape target: `creg-phase-c-r7-o1:8080/metrics`, every 15 seconds. Target health was `up`, its last scrape was `2026-09-27T09:19:30.36599193Z`, and `lastError` was empty.
- Target labels: `chain_id=creg-phase-c-testnet-2026-09-26-r7`, `network_genesis_hash=1852b9a86dd3b53e7b2de4a774723a4fe48e862f9c61d14e5917b42b21055234`, `role=observer`, `metrics_source=explorer-primary`.
- A Prometheus instant query filtered by the exact chain, signed-spec genesis identity and primary source returned `creg_chain_tip_height=14`.
- An earlier range query returned six actual samples at 15-second intervals, all at height 14, with those R7 labels. No values were backfilled.
- Final live recheck at `2026-09-27T10:48:23Z`: Prometheus health endpoint was reachable, the R7 scrape target remained `up=1`, and a five-minute range query returned 21 genuine `creg_chain_tip_height=14` samples at 15-second intervals with the exact R7 identity labels. This confirms ongoing retained history; restart-survival has not yet been exercised.
- `promtool check config /etc/prometheus/prometheus.yml` passed.
- The existing compose profile retains 14 days. Explorer choices are limited to 7 days, leaving a retention margin while disk growth is measured.
- Alertmanager was not started. The running observer container was not restarted; it was attached to the existing `monitoring_default` Docker network so Prometheus can scrape it by container name.

## Source and build status

The local source now adds a public, fixed-series history endpoint backed by Prometheus range queries filtered to the node's configured chain ID and signed network genesis identity. It returns an explicit 503 while the collector is unavailable or has not yet stored samples. The node's real `/metrics` output includes chain-authoritative total stake. The web Metrics page and CLI TUI consume retained samples and expose collector unavailability.

Builds completed:

- `cargo check --locked -p chain-registry-node -p resolver` passed under Ubuntu 24.04 WSL with Rust 1.98.1, LLVM/libclang 18 and `protoc` installed. This covers the publication-proof retention repair and node history API source.
- `cargo check --locked -p chain-registry-cli` passed in the same environment.
- `npm run build` passed for the explorer. Vite reported that one minified bundle exceeds 500 kB.
- The first full `cargo build --locked -p chain-registry-node -p resolver -p chain-registry-cli` attempt in WSL stopped with I/O and read-only-filesystem errors while writing its target directory. The retry completed successfully in the isolated `creg-node:recovery-diagnostic-builder-r18` Linux container, with the source mounted read-only and output directed to `E:\creg-phase-c-build-20260927`. The builder provided libclang 17, LLVM and `protoc`; Cargo compiled the node, resolver and CLI in 77m55s. The `creg-node` and `creg` executables are at `E:\creg-phase-c-build-20260927\target\debug\creg-node` and `E:\creg-phase-c-build-20260927\target\debug\creg`. Cargo emitted a future-incompatibility notice for `sqlx-postgres v0.7.4`, but exited 0. These binaries have not been staged to the fleet.

## Remaining integration gate

The currently deployed candidate node still runs the R22 image; its `/v1/public/metrics/history?range=5m` route returned HTTP 404 at capture. The newly compiled API and explorer source have not been rolled out to candidate services. Next, stage and review the candidate image, then verify real range results in the node API, web explorer and CLI, including retained samples after a collector/API restart. Public routing stays on the legacy chain until all cutover gates pass.
