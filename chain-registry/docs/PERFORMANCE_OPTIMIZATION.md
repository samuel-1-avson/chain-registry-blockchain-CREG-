# Performance Optimization Guide

## Current Baseline

| Metric | Before | After | Improvement |
|--------|--------|-------|-------------|
| Block Interval | 5s | **2s** | 60% faster |
| Vote Timeout | 30s | **10s** | 67% faster |
| Pipeline Poll | 2s | **1s** | 2x more responsive |
| Target Throughput | 6 pkg/min | **30 pkg/min** | 5x increase |

## Changes Made

### 1. Block Interval Reduction

**File:** `.env.testnet`
```bash
# Old
CREG_BLOCK_INTERVAL=5

# New
CREG_BLOCK_INTERVAL=2
```

**Impact:** More frequent block production, faster finality.

### 2. Vote Collection Timeout

**File:** `crates/node/src/validator_pipeline.rs`
```rust
// Old
const POLL_INTERVAL_SECS: u64 = 2;
for _ in 0..60 { // 30 seconds

// New
const POLL_INTERVAL_SECS: u64 = 1;
const VOTE_TIMEOUT_SECS: u64 = 10;
for _ in 0..max_iterations { // 10 seconds
```

**Impact:** Faster consensus failure detection, higher throughput.

### 3. PostgreSQL Sync Worker Fix

**File:** `crates/db-sync/src/lib.rs`
```rust
// Fixed: Added missing hash field and corrected parameter order
.bind(block.header.height as i64)
.bind(&block.header.hash)          // Was missing!
.bind(&block.header.prev_hash)
.bind(&block.header.merkle_root)
.bind(&block.header.proposer_id)
.bind(block.header.timestamp)
```

**Impact:** Blocks now sync correctly to PostgreSQL.

## Deployment

```bash
# 1. Rebuild containers with fixes
docker-compose -f docker-compose.testnet.yml down
docker-compose -f docker-compose.testnet.yml build --no-cache
docker-compose -f docker-compose.testnet.yml up -d

# 2. Verify performance improvements
curl http://localhost:3001/stats

# 3. Run stress test
./scripts/stress_test.ps1
```

## Expected Results

### Block Production
```
Before: Block 1000 @ 12:00:00
        Block 1001 @ 12:00:05  (5s interval)
        Block 1002 @ 12:00:10

After:  Block 1000 @ 12:00:00
        Block 1001 @ 12:00:02  (2s interval)
        Block 1002 @ 12:00:04
```

### Package Throughput
```
Before: ~6 packages/minute (limited by 5s block time)
After:  ~30 packages/minute (2s block time + faster consensus)
```

### Vote Latency
```
Before: Average 567ms (30s timeout buffer)
After:  Target <200ms (10s timeout buffer)
```

## Monitoring

### Key Metrics
```bash
# Block time
curl http://localhost:3001/metrics | grep block_interval

# Vote latency
curl http://localhost:3001/metrics | grep vote_latency

# Package throughput
curl http://localhost:3001/metrics | grep packages_per_minute
```

### Alerts
```yaml
- alert: HighBlockTime
  expr: block_interval > 3
  for: 1m
  
- alert: LowThroughput
  expr: packages_per_minute < 20
  for: 5m
```

## Further Optimizations (Future)

### 1. Batch Vote Processing
Process multiple votes in a single database transaction.

### 2. Parallel Validation
Run static analysis, sandbox, and AI scanning in parallel.

### 3. Caching Layer
Cache validation results for identical packages.

### 4. Connection Pooling
Optimize PostgreSQL connection pool size.

### 5. Pre-compiled Circuits
Use pre-generated ZK proofs for common validation patterns.

## Benchmarks

Run these to verify improvements:

```bash
# Single package benchmark
creg benchmark publish --package npm:test@1.0.0

# Batch benchmark
creg benchmark batch --count 100 --concurrency 10

# Load test
./scripts/load_test.sh --duration 60s --rate 50
```

## Troubleshooting

### "Consensus timeout" errors
- Check validator network connectivity
- Verify all validators are online
- Increase `CREG_VOTE_TIMEOUT_MS` if needed

### PostgreSQL connection errors
- Verify `CREG_PG_URL` is correct
- Check PostgreSQL is accepting connections
- Review connection pool settings

### Low throughput
- Check CPU usage across validators
- Verify IPFS is not the bottleneck
- Monitor network latency between validators

---

*Last updated: 2026-04-02*
*Target: 2s block time, <200ms vote latency, 30 pkg/min throughput*
