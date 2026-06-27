# Chain spec static server

Serves the **signed** `chain-spec.sepolia.json` (+ `.sig`) over HTTP so nodes can verify network parameters at boot.

## Files

| File | Purpose |
|------|---------|
| `chain-spec.sepolia.json` | Copy of `../chain-spec.sepolia.json` |
| `chain-spec.sepolia.json.sig` | Detached Ed25519 signature |
| `nginx.conf` | Static file server |
| `docker-compose.yml` | Local spec-server container |

## Update after chain-spec change

```powershell
cd chain-registry\testnet
.\finalize-sepolia-spec.ps1
copy chain-spec.sepolia.json spec-server\chain-spec.sepolia.json
copy chain-spec.sepolia.json.sig spec-server\chain-spec.sepolia.json.sig
```

Full procedure: [docs/TESTNET_SEPOLIA_RUNBOOK.md](../../../docs/TESTNET_SEPOLIA_RUNBOOK.md).

## Verify independently

```bash
cargo run --example verify_chain_spec --package common -- \
  testnet/chain-spec.sepolia.json \
  $(cat testnet/chain-spec.sepolia.json.sig)
```