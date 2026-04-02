# Mainnet Deployment Checklist

## Overview

This checklist covers the complete deployment process for Chain Registry mainnet launch.

**Estimated Timeline:** 2-3 weeks  
**Team Required:** DevOps, Smart Contract Engineers, Security Auditors  
**Risk Level:** High (irreversible operations)  

---

## Phase 1: Pre-Deployment (Week 1)

### 1.1 Security Audits ✅ REQUIRED

| Item | Status | Assignee | Notes |
|------|--------|----------|-------|
| Smart Contract Audit | ⬜ | Trail of Bits/OpenZeppelin | All Solidity contracts |
| ZK Circuit Formal Verification | ⬜ | Specialized firm | DoubleSignProof.circom |
| Cryptographic Review | ⬜ | External cryptographer | Ed25519, threshold encryption |
| Penetration Testing | ⬜ | Security firm | APIs, P2P, nodes |

**Deliverables:**
- [ ] Audit reports with severity ratings
- [ ] Remediation plan for critical/high findings
- [ ] Final sign-off from security team

### 1.2 Testing ✅ REQUIRED

| Test Type | Status | Coverage Target |
|-----------|--------|-----------------|
| Unit Tests | ⬜ | >90% |
| Integration Tests | ⬜ | All critical paths |
| Fuzz Testing | ⬜ | Smart contracts |
| Chaos Testing | ⬜ | Node failures, network partitions |
| Load Testing | ⬜ | 100x expected traffic |
| Economic Simulation | ⬜ | Attack scenarios |

**Deliverables:**
- [ ] Test reports with metrics
- [ ] Known issues document
- [ ] Testnet run for 30 days without critical issues

### 1.3 Documentation ✅ REQUIRED

| Document | Status | Location |
|----------|--------|----------|
| Architecture Overview | ⬜ | docs/ARCHITECTURE.md |
| API Reference | ⬜ | docs/API.md |
| Operator Guide | ⬜ | docs/OPERATOR.md |
| Security Model | ✅ | docs/SECURITY.md |
| Emergency Procedures | ⬜ | docs/EMERGENCY.md |
| Upgrade Procedures | ⬜ | docs/UPGRADES.md |

---

## Phase 2: Infrastructure Setup (Week 1-2)

### 2.1 Validator Node Setup

#### Hardware Requirements
```yaml
Minimum per validator:
  CPU: 8 cores (16 threads)
  RAM: 32 GB
  Storage: 1 TB NVMe SSD
  Network: 1 Gbps symmetrical
  Location: Diverse geographic regions

Recommended:
  CPU: 16 cores
  RAM: 64 GB
  Storage: 2 TB NVMe SSD
  Network: 10 Gbps
  Redundancy: Dual power, dual network
```

#### Node Checklist
- [ ] Provision 10+ validator servers
- [ ] Distribute across 3+ continents
- [ ] Configure firewalls (allow 4001, 8080, 50051)
- [ ] Set up monitoring (Prometheus + Grafana)
- [ ] Configure log aggregation (ELK/Loki)
- [ ] Set up alerting (PagerDuty/Opsgenie)

### 2.2 Network Infrastructure

| Component | Provider | SLA | Status |
|-----------|----------|-----|--------|
| Bootstrap DNS | Cloudflare | 99.99% | ⬜ |
| IPFS Gateway | Pinata/Infura | 99.9% | ⬜ |
| Ethereum RPC | Alchemy/QuickNode | 99.9% | ⬜ |
| PostgreSQL | AWS RDS/Cloud SQL | 99.95% | ⬜ |
| Backup Storage | AWS S3/GCS | 99.99% | ⬜ |

### 2.3 Monitoring Stack

```yaml
Metrics:
  - Prometheus (collection)
  - Grafana (visualization)
  - Node Exporter (system metrics)
  
Logs:
  - Loki (aggregation)
  - Promtail (collection)
  
Alerts:
  - Alertmanager (routing)
  - PagerDuty (notifications)
  
Dashboards:
  - Network health
  - Validator performance
  - Package throughput
  - Economic metrics
```

---

## Phase 3: Contract Deployment (Week 2)

### 3.1 Pre-Deployment Checks

- [ ] Verify compiler version (Solidity 0.8.19+)
- [ ] Verify optimizer settings (200 runs)
- [ ] Check contract bytecode against audited version
- [ ] Prepare deployment scripts
- [ ] Test deployment on forked mainnet
- [ ] Prepare multisig wallets

### 3.2 Deployment Order

```
Day 1: Infrastructure Contracts
  1. CREGToken (ERC20)
  2. TimelockController (2-day delay)
  3. ProxyAdmin
  
Day 2: Core Protocol
  4. Registry (with proxy)
  5. Staking (with proxy)
  6. Reputation (with proxy)
  
Day 3: Advanced Features
  7. VRFCoordinator (with proxy)
  8. IPFSPinningRewards (with proxy)
  9. ZKSlashingVerifier
  
Day 4: Governance
  10. GovernanceV2 (with proxy)
  11. InsurancePool (with proxy)
```

### 3.3 Deployment Parameters

```solidity
// CREGToken
INITIAL_SUPPLY = 100_000_000 ether;  // 100M CREG

// Staking
MIN_VALIDATOR_STAKE = 10_000 ether;   // 10K CREG
MIN_PINNER_STAKE = 1_000 ether;       // 1K CREG

// Rewards
BLOCK_REWARD = 0.5 ether;             // 0.5 CREG/block
PINNING_BASE_RATE = 0.01 ether;       // 0.01 CREG/GB/day

// Slashing
DOUBLE_SIGN_SLASH = 1_000 ether;      // 1000 CREG
PINNING_FAILURE_SLASH = 0.01 ether;   // 1% of stake
```

### 3.4 Post-Deployment Verification

- [ ] Verify all contracts on Etherscan
- [ ] Verify proxy implementations
- [ ] Test admin functions via multisig
- [ ] Deposit initial liquidity to Insurance Pool
- [ ] Distribute initial CREG to validators

---

## Phase 4: Validator Onboarding (Week 2-3)

### 4.1 Genesis Validator Set

| # | Organization | Location | Stake | Status |
|---|--------------|----------|-------|--------|
| 1 | Chain Registry Foundation | US-East | 100K | ⬜ |
| 2 | Node Operator A | EU-West | 50K | ⬜ |
| 3 | Node Operator B | Asia-East | 50K | ⬜ |
| 4 | Community Validator 1 | US-West | 25K | ⬜ |
| 5 | Community Validator 2 | EU-Central | 25K | ⬜ |
| 6 | Community Validator 3 | Asia-South | 25K | ⬜ |
| 7 | Community Validator 4 | South America | 25K | ⬜ |
| 8 | Community Validator 5 | Africa | 25K | ⬜ |
| 9 | Community Validator 6 | Oceania | 25K | ⬜ |
| 10 | Community Validator 7 | Middle East | 25K | ⬜ |

### 4.2 Validator Setup Checklist

For each validator:
- [ ] Generate Ed25519 keypair
- [ ] Fund Ethereum address with ETH for gas
- [ ] Stake minimum CREG (10K)
- [ ] Deploy node with monitoring
- [ ] Join P2P network
- [ ] Verify connectivity to 9+ peers
- [ ] Register in validator set contract

### 4.3 Network Bootstrap

```bash
# 1. Start bootstrap node
node-1> creg start --bootstrap --listen /ip4/0.0.0.0/tcp/4001

# 2. Add remaining validators
node-2..10> creg start --seeds /dns4/bootstrap.creg.network/tcp/4001

# 3. Verify mesh connectivity
> creg net peers
# Should show 9+ connected peers per node

# 4. Verify consensus
> creg chain height
# All nodes should show same height
```

---

## Phase 5: Launch Day (Week 3)

### 5.1 Pre-Launch (T-24 hours)

- [ ] All validators online and synced
- [ ] Monitoring dashboards verified
- [ ] Alert channels tested
- [ ] Emergency contacts confirmed
- [ ] Social media announcements scheduled
- [ ] Bug bounty program active

### 5.2 Launch Sequence (T-0)

```
T-60 min: Final health check all systems
T-30 min: Enable public RPC endpoints
T-15 min: Open firewall to public traffic
T-10 min: Activate monitoring alerts
T-5 min:  Final validator consensus check
T-0:      ENABLE PACKAGE PUBLISHING
T+5 min:  Monitor first packages
T+1 hour: Post-launch status report
```

### 5.3 Launch Day Monitoring

| Metric | Target | Alert Threshold |
|--------|--------|-----------------|
| Block Time | 2s | >5s |
| Validator Participation | 100% | <80% |
| Package Success Rate | >95% | <80% |
| P2P Peer Count | 9+ | <5 |
| API Response Time | <200ms | >1s |
| Contract Gas Price | <100 gwei | >500 gwei |

---

## Phase 6: Post-Launch (Week 3+)

### 6.1 Week 1 Monitoring

- [ ] Daily standup with validator operators
- [ ] Monitor economic parameters
- [ ] Track package volume
- [ ] Watch for security incidents
- [ ] Collect user feedback

### 6.2 Month 1 Activities

- [ ] Weekly performance reports
- [ ] Validator set expansion (10 → 20)
- [ ] First contract upgrade (if needed)
- [ ] Bug bounty payouts (if any)
- [ ] Community governance proposals

### 6.3 Emergency Procedures

#### Contract Pause
```solidity
// Multi-sig required (5 of 9)
function emergencyPause() external onlyMultisig {
    registry.pause();
    staking.pause();
    pinning.pause();
}
```

#### Validator Replacement
```bash
# 1. Identify faulty validator
# 2. Consensus from 7+ validators
# 3. Execute removal via governance
# 4. Add replacement validator
```

#### Rollback Procedure
```bash
# 1. Halt all validators
# 2. Restore from last known good backup
# 3. Replay blocks up to safe height
# 4. Restart with patched version
```

---

## Security Checklist

### Pre-Launch
- [ ] All contracts audited
- [ ] Keys stored in hardware security modules (HSMs)
- [ ] Multisig wallets configured and tested
- [ ] Emergency contacts verified
- [ ] Incident response plan documented
- [ ] Insurance coverage confirmed
- [ ] Bug bounty program funded

### Post-Launch
- [ ] Monitor for unusual activity
- [ ] Daily security scans
- [ ] Weekly access reviews
- [ ] Monthly penetration tests

## Rollback Criteria

**Immediate Rollback Required:**
- Critical security vulnerability discovered
- Funds at risk
- Consensus failure
- >50% validator downtime

**Rollback Procedure:**
1. Execute emergency pause
2. Assess impact
3. Notify community
4. Deploy fix
5. Resume with monitoring

---

## Success Metrics

| Metric | Target | Measurement |
|--------|--------|-------------|
| Uptime | 99.9% | 30-day rolling |
| Package Volume | 1000/day | Daily count |
| Validator Participation | >90% | Per epoch |
| Security Incidents | 0 | Critical/High |
| Community Growth | 10% MoM | Active users |

---

## Contacts

| Role | Name | Contact | Escalation |
|------|------|---------|------------|
| Tech Lead | | | |
| Security Lead | | | |
| DevOps Lead | | | |
| Community Manager | | | |
| Emergency Hotline | | | |

---

*Checklist Version: 1.0*  
*Last Updated: 2026-04-02*  
*Next Review: Launch Day - 1 week*