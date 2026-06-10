# Observability (REM-211)

Prometheus, Grafana, Loki, and Tempo configs for Chain Registry.

## Sepolia single-node (Option A reuse)

When `creg-node` runs on the host at `http://localhost:8090` (see `testnet/run-sepolia-reuse.ps1 -StartNode`):

1. Confirm metrics: `curl http://localhost:8090/metrics`
2. Start Prometheus with the local profile:

```powershell
cd chain-registry
docker run --rm -d --name creg-prom-sepolia -p 9090:9090 `
  -v "${PWD}/observability/prometheus.sepolia-local.yml:/etc/prometheus/prometheus.yml:ro" `
  prom/prometheus:v2.51.0 `
  --config.file=/etc/prometheus/prometheus.yml
```

3. Open http://localhost:9090/targets — `creg_node_sepolia` should be **UP**.

4. Optional full stack (Grafana + Loki + alerts): from repo root with main compose files, swap Prometheus config to `prometheus.testnet.yml` for multi-node Docker testnet.

### Validate config (offline)

The Prometheus image entrypoint is `prometheus`, not `promtool`. Use:

```powershell
cd chain-registry
docker run --rm --entrypoint promtool `
  -v "${PWD}/observability/prometheus.sepolia-local.yml:/etc/prometheus/prometheus.yml:ro" `
  prom/prometheus:v2.51.0 check config /etc/prometheus/prometheus.yml
```

### Key metrics (`GET /metrics`)

**Local CREG chain** (observer node with only genesis is normal):

| Metric | Typical idle | Meaning |
|--------|----------------|---------|
| `creg_chain_tip_height` | `0` | Tip block height index (not Sepolia) |
| `creg_chain_blocks_stored` | `1` | Blocks in DB (genesis counts as 1) |
| `creg_chain_height` / `creg_block_count` | same | Deprecated aliases |

**L1 validator set** (Sepolia staking sync — use for ops alerts):

| Metric | Meaning |
|--------|---------|
| `creg_validator_set_sync_state_code` | `4` = synced; `1` = syncing; `3` = degraded |
| `creg_validator_set_sync_last_finalized_source_block` | Last applied Sepolia block |
| `creg_validator_set_sync_info{state="synced",...}` | Labels mirror `/v1/health` |
| `creg_validator_set_sync_has_error` | `1` if `last_error` is set |

Example alert: `creg_validator_set_sync_state_code != 4` for 5m (when sync enabled).

## Testnet (10 validators)

See comments in `prometheus.testnet.yml` and `docker-compose.observability.yml`.

## Dashboards

- `grafana-dashboard.json` — import into Grafana (port 3000 when observability compose is up).
- `alerts.yml` — used with default `prometheus.yml`.

## Acceptance (REM-211)

- [ ] `/metrics` returns Prometheus text from running node
- [ ] `prometheus.sepolia-local.yml` target healthy for `:8090`
- [ ] Grafana dashboard shows node block/consensus metrics (optional)
