# $CREG Token Economics

**Version:** 1.0  
**Date:** March 30, 2026

---

## Overview

$CREG is the governance and utility token for Chain Registry. It enables decentralized governance, incentivizes validators, and powers the package insurance system.

## Token Parameters

| Parameter | Value |
|-----------|-------|
| **Name** | Chain Registry Token |
| **Symbol** | CREG |
| **Type** | ERC-20 (with voting extensions) |
| **Max Supply** | 1,000,000,000 CREG |
| **Initial Supply** | 200,000,000 CREG (20%) |
| **Inflation** | 2% per year |
| **Decimals** | 18 |

## Token Distribution

```
┌─────────────────────────────────────────────────────────────┐
│                    $CREG Token Distribution                  │
├─────────────────────────────────────────────────────────────┤
│                                                              │
│  Team (20%)          ████████████████████  200M CREG        │
│  └─ 4-year vesting, 1-year cliff                          │
│                                                              │
│  Investors (15%)     ███████████████       150M CREG        │
│  └─ 2-year vesting, 6-month cliff                         │
│                                                              │
│  Community (25%)     █████████████████████ 250M CREG        │
│  └─ Staking rewards, validator incentives                  │
│                                                              │
│  Treasury (40%)      ████████████████████████████████████   │
│  └─ Ecosystem development, grants, insurance pool          │
│                                                              │
└─────────────────────────────────────────────────────────────┘
```

### Detailed Allocation

| Category | Allocation | Amount | Vesting | Purpose |
|----------|------------|--------|---------|---------|
| Team | 20% | 200M | 4yr vest, 1yr cliff | Core team compensation |
| Investors | 15% | 150M | 2yr vest, 6mo cliff | Seed & Series A |
| Community Rewards | 25% | 250M | Dynamic | Staking & incentives |
| Treasury | 40% | 400M | At launch | Ecosystem development |

## Inflation Model

### Annual Inflation: 2%

- **Minted to:** Treasury
- **Purpose:** Protocol sustainability
- **Trigger:** Once per year (governance callable)
- **Max supply cap:** 1B CREG (hard cap)

### Inflation Schedule

| Year | Inflation | New Supply | Total Supply |
|------|-----------|------------|--------------|
| 0 | - | - | 1,000,000,000 |
| 1 | 2% | 20,000,000 | 1,020,000,000 |
| 2 | 2% | 20,400,000 | 1,040,400,000 |
| 5 | 2% | 21,648,643 | 1,082,432,160 |
| 10 | 2% | 23,783,464 | 1,189,173,217 |

*Note: Inflation stops when total supply reaches 1B cap*

## Utility

### 1. Governance Voting

```
Raw Token Balance → Quadratic Voting Power
     100 CREG    →    10 votes
   10,000 CREG   →   100 votes
 1,000,000 CREG  → 1,000 votes
```

**Quadratic Formula:** `voting_power = sqrt(token_balance)`

**Benefits:**
- Reduces whale dominance
- Encourages broader participation
- More democratic outcomes

### 2. Staking

| Staking Period | APY | Multiplier |
|----------------|-----|------------|
| No lock | 5% | 1x |
| 1 month | 8% | 1.5x |
| 3 months | 12% | 2x |
| 6 months | 15% | 2.5x |
| 12 months | 20% | 3x |

**Staking Benefits:**
- Earn yield from protocol fees
- Increased voting power
- Eligibility for airdrops
- Validator status (100K+ CREG)

### 3. Package Insurance

**Premium Payments:**
- All premiums paid in CREG
- Reduces token velocity
- Creates buy pressure

**Claim Payouts:**
- Payouts in CREG
- Instant settlement
- No fiat intermediary

**Risk-Based Pricing:**

| Risk Level | Premium Rate | Example |
|------------|--------------|---------|
| Low | 0.5% - 1% | Established packages |
| Medium | 1% - 3% | Newer packages |
| High | 3% - 10% | Experimental packages |

### 4. Fee Discounts

| CREG Held | ZK Verification | Cross-Chain | Insurance |
|-----------|-----------------|-------------|-----------|
| 1,000+ | 5% off | 5% off | 5% off |
| 10,000+ | 10% off | 10% off | 10% off |
| 100,000+ | 25% off | 25% off | 25% off |
| 1,000,000+ | 50% off | 50% off | 50% off |

## Revenue Model

### Protocol Fees

| Service | Fee | Destination |
|---------|-----|-------------|
| Package Submission | 0.001 ETH | Treasury |
| ZK Verification | 0.001 ETH | Treasury + Stakers |
| Cross-Chain Sync | Variable | Bridge + Treasury |
| Insurance Premium | 0.5-10% | Insurance Pool |
| Private Registry | 0.01 ETH | Treasury |

### Fee Distribution

```
┌─────────────────────────────────────────────────────────────┐
│                      Fee Distribution                        │
├─────────────────────────────────────────────────────────────┤
│                                                              │
│  Protocol Revenue                                           │
│       │                                                     │
│       ▼                                                     │
│  ┌──────────────┐                                          │
│  │ Treasury 50% │──▶ Development, Marketing, Operations    │
│  └──────────────┘                                          │
│       │                                                     │
│  ┌──────────────┐                                          │
│  │ Stakers 30%  │──▶ Distributed to CREG stakers           │
│  └──────────────┘                                          │
│       │                                                     │
│  ┌──────────────┐                                          │
│  │ Insurance 20%│──▶ Insurance pool reserve               │
│  └──────────────┘                                          │
│                                                              │
└─────────────────────────────────────────────────────────────┘
```

## Governance

### Proposal Requirements

| Parameter | Value |
|-----------|-------|
| Proposal Threshold | 100,000 CREG (quadratic) |
| Voting Delay | 1 block (~12s) |
| Voting Period | 40,320 blocks (~7 days) |
| Execution Delay | 2 days |
| Quorum | 4% of total supply |

### Delegation

- Users can delegate voting power
- Delegation is revocable
- Vote tracking via checkpoints
- Gasless delegation via signatures

### Automated Parameter Adjustment

Governance can set automated parameter changes:

```solidity
// Example: Gradually increase staking rewards
proposeAutoAdjustment(
    "staking_apy",
    targetValue: 2500,  // 25% APY
    changeRate: 10,     // 0.1% per day max
    "Gradually increase staking APY"
);
```

## Insurance Pool Economics

### Pool Structure

| Source | Percentage | Description |
|--------|------------|-------------|
| Premiums | 60% | Main revenue source |
| Slashings | 25% | From malicious publishers |
| Treasury | 15% | Protocol subsidy |

### Solvency Requirements

| Metric | Minimum | Target | Maximum |
|--------|---------|--------|---------|
| Solvency Ratio | 100% | 150% | 300% |
| Utilization | 30% | 60% | 80% |
| Claim Reserve | 20% | 30% | 50% |

### Risk Management

**Dynamic Pricing:**
```
Premium = Base Rate × Risk Multiplier × Time Factor

Where:
- Base Rate = 1%
- Risk Multiplier = f(package_age, dependencies, vulnerabilities)
- Time Factor = f(market_conditions, pool_health)
```

## Token Flow

```
┌─────────────────────────────────────────────────────────────────────┐
│                        $CREG Token Flow                              │
├─────────────────────────────────────────────────────────────────────┤
│                                                                      │
│  USERS                          PROTOCOL                           │
│  ┌──────────┐                  ┌──────────┐                         │
│  │ Purchase │───CREG──────────▶│ Insurance│                         │
│  │ Insurance│                  │   Pool   │                         │
│  └──────────┘                  └────┬─────┘                         │
│       │                             │                               │
│       │                             │ Payouts                       │
│       │                             ▼                               │
│  ┌──────────┐                  ┌──────────┐                        │
│  │  Stake   │◀───CREG─────────│  Claims  │                        │
│  │  Tokens  │                  │          │                        │
│  └────┬─────┘                  └──────────┘                        │
│       │                                                              │
│       │ Staking Rewards                                             │
│       ▼                                                              │
│  ┌──────────┐                                                       │
│  │  Voting  │───Governance────▶ Protocol Changes                   │
│  │  Power   │                                                       │
│  └──────────┘                                                       │
│                                                                      │
│  EXTERNAL BUY PRESSURE                                             │
│  ┌──────────┐                  ┌──────────┐                        │
│  │  Users   │───ETH/USD───────▶│  DEX/AMM │───CREG──────────┐     │
│  │ Want Ins │                  │          │                 │     │
│  │ or Disc  │                  └──────────┘                 │     │
│  └──────────┘                                              ▼     │
│                                                     ┌──────────┐  │
│                                                     │ Circulating│  │
│                                                     │   Supply   │  │
│                                                     └──────────┘  │
│                                                                      │
└─────────────────────────────────────────────────────────────────────┘
```

## Value Accrual

### Demand Drivers

1. **Insurance Demand** - Premiums must be paid in CREG
2. **Governance** - Voting requires holding CREG
3. **Staking** - Yield attracts long-term holders
4. **Fee Discounts** - Holding CREG reduces costs
5. **Validator Status** - Requires significant CREG stake

### Supply Dynamics

| Factor | Effect | Magnitude |
|--------|--------|-----------|
| Staking | Removes from circulation | 30-50% |
| Insurance Pool | Locked in contracts | 10-20% |
| Treasury | Strategic reserves | 20-30% |
| Annual Inflation | New supply | +2%/year |

### Price Support Mechanisms

1. **Buybacks:** Protocol fees used to buy CREG from market
2. **Burn:** 50% of insurance profits burned
3. **Staking:** Reduces sell pressure
4. **Utility:** Required for all core functions

## Security Considerations

### Economic Attacks

| Attack | Mitigation |
|--------|------------|
| Flash loan voting | Checkpoint-based voting (no flash loans) |
| Governance takeover | Quadratic voting + high threshold |
| Insurance fraud | Multi-sig claim resolution + evidence |
| Pump & dump | Vesting schedules + staking locks |

### Emergency Procedures

1. **Circuit Breaker:** Pause insurance claims if solvency < 100%
2. **Parameter Freeze:** Emergency governance can freeze parameters
3. **Upgrade Path:** Timelocked upgrades with 7-day delay

## Roadmap

### Phase 1 (Current)
- ✅ Token contract deployed
- ✅ Basic governance functional
- ✅ Staking mechanism live

### Phase 2 (Q2 2026)
- Insurance pool launch
- Cross-chain token bridges
- DEX listings

### Phase 3 (Q3-Q4 2026)
- Governance parameter optimization
- Advanced insurance products
- Token buyback program

### Phase 4 (2027+)
- Full decentralization
- Community treasury control
- Protocol fee optimization

---

**Document Version:** 1.0  
**Last Updated:** March 30, 2026  
**Author:** Chain Registry Economics Team
