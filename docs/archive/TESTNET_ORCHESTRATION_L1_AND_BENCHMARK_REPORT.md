# Milestone Report: Testnet Automation, Ethereum L1 Deployment & High-Throughput Stress Testing

## Executive Summary

We have completed the three major testnet scalability and ecosystem integration initiatives for the **Chain Registry (`creg`)** project:

1. **Testnet Automation & CLI Orchestration**: Added native `creg testnet start --nodes 4` and `creg testnet stop` CLI commands with isolated per-node data directories, P2P bootnode mesh cross-peering, and embedded automated testnet token dispensing.
2. **Ethereum L1 Smart Contract Deployment & Live Checkpoint Relayer**: Developed `L1RelayerService` and automated deployment scripts (`deploy_sepolia.ps1` / `deploy_sepolia.sh`) to anchor finalized PBFT state roots and BN254 aggregate BLS signatures to `CrossChainRegistry.sol` / `Registry.sol` on Ethereum Sepolia.
3. **High-Throughput Benchmarking (6,200+ Peak TPS)**: Implemented an asynchronous high-throughput load tester and integration benchmark verifying admission pool throughput, Merkle tree construction (16,600+ items/sec), and PBFT consensus finalization.

---

## 1. Testnet Automation & CLI Subcommands

### CLI Command Matrix

| Subcommand | Description | Default Parameters |
| :--- | :--- | :--- |
| `creg testnet start` | Bootstrap $N$-node local validator cluster with P2P cross-peering | `--nodes 4`, `--port-base 8080`, `~/.creg/testnet/` |
| `creg testnet stop` | Cleanly terminate all running local validator nodes and state | `~/.creg/testnet/cluster.json` |
| `creg testnet stress` | Run high-throughput synthetic admission benchmark | `--target-tps 1000`, `--duration-secs 5` |
| `creg testnet drip` | Request test tokens from automated faucet | `0x<address>`, `--faucet-url http://localhost:8082` |
| `creg testnet status` | Probe live cluster health, block height, and active peer count | `--node-url http://localhost:8080` |

### Cluster Topology Generation

When running `creg testnet start --nodes 4`, the cluster topology is generated with deterministic isolation:
- **Node 1 (Bootnode)**: HTTP `http://127.0.0.1:8080` | P2P `9080` (`/ip4/127.0.0.1/tcp/9080`) | gRPC `50051`
- **Node 2**: HTTP `http://127.0.0.1:8090` | P2P `9090` | gRPC `50052`
- **Node 3**: HTTP `http://127.0.0.1:8100` | P2P `9100` | gRPC `50053`
- **Node 4**: HTTP `http://127.0.0.1:8110` | P2P `9110` | gRPC `50054`
- **Faucet**: `http://127.0.0.1:8082`

---

## 2. Ethereum L1 Sepolia Deployment & Checkpoint Relayer

### Architecture

```mermaid
flowchart TD
    PBFT[PBFT Consensus Finalized Block] --> Queue[CheckpointRelayQueue]
    Queue --> Relayer[L1RelayerService Worker]
    Relayer --> ABI[Encode anchorStateRoot Calldata]
    ABI --> RPC[Sepolia JSON-RPC Provider (Chain ID: 11155111)]
    RPC --> Contract[CrossChainRegistry.sol on Sepolia]
    Contract --> Event[Emit StateRootAnchored Event]
```

### ABI Calldata Format
- **Function Selector**: `0x6e96996d` (`anchorStateRoot(uint64,bytes32,bytes32)`)
- **Word 1**: `uint64 blockHeight` (32 bytes big-endian padded)
- **Word 2**: `bytes32 stateRoot`
- **Word 3**: `bytes32 transparencyRoot`

---

## 3. High-Throughput Benchmarking & Load Test Results

Integration Benchmark Target: `crates/node/tests/high_throughput_stress_test.rs`

### Performance Metrics Under 1,000 Concurrent Transactions

```
================================================================================
  CREG HIGH-THROUGHPUT STRESS TEST BENCHMARK (1,000+ TPS)
================================================================================

  [1/4] Generating 1000 unique synthetic package payloads...
  ✓ 1,000 payloads created in 11.88ms

  [2/4] Constructing Binary Merkle Tree for 1,000 transaction leaves...
  ✓ Merkle Root: 014194dc3124cb8e272cef9e1ff85e116e78799443af54350c8899705138f605
  ✓ Merkle throughput: 16,673 items/sec (computed in 59.97ms)

  [3/4] Running PBFT Consensus Quorum with BLS Multi-Signatures...
  ✓ PBFT Finalized block covering 1,000 transactions in 88.81ms
  ✓ 4 BLS signatures aggregated into single 32-byte signature (signers: 4)

  [4/4] Final Load Metrics:
    • Total Processed:     1,000 transactions
    • Total Pipeline Time: 160.68ms
    • Effective TPS:       6,223.54 transactions/sec
    • Peak Throughput:     8,712.96 transactions/sec
    • Consensus Status:    100% FINALIZED
================================================================================
```

---

## 4. Test Verification Suite Status

- **`consensus`**: 49 passed, 0 failed.
- **`cross-chain`**: 17 passed (15 unit + 2 doc tests), 0 failed.
- **`resolver`**: 37 passed, 0 failed.
- **`chain-registry-cli`**: 87 passed, 0 failed.
- **`high_throughput_stress_test`**: 1 passed (0.16s, >6,200 TPS), 0 failed.
