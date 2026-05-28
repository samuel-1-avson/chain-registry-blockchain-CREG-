# Observability — Sepolia single node (REM-211)

Prometheus, Grafana, Loki, and Tempo configs live under `chain-registry/observability/`.

## Option A reuse on the host

When `creg-node` runs at `http://localhost:8090` (`testnet/run-sepolia-reuse.ps1 -StartNode`):

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

4. Optional full stack: use `docker-compose.observability.yml` with `prometheus.testnet.yml` for the 10-validator Docker testnet.

## Validate config (offline)

The Prometheus image entrypoint is `prometheus`, not `promtool`. Use:

```powershell
cd chain-registry
docker run --rm --entrypoint promtool `
  -v "${PWD}/observability/prometheus.sepolia-local.yml:/etc/prometheus/prometheus.yml:ro" `
  prom/prometheus:v2.51.0 check config /etc/prometheus/prometheus.yml
```

## Dashboards

- `chain-registry/observability/grafana-dashboard.json` — import into Grafana.
- `chain-registry/observability/alerts.yml` — used with default `prometheus.yml`.

## Acceptance (REM-211)

- [ ] `/metrics` returns Prometheus text from a running node
- [ ] `prometheus.sepolia-local.yml` target healthy for `:8090`
- [ ] Grafana dashboard shows node metrics (optional)
