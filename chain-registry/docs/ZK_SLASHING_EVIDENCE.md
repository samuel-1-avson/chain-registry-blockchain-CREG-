# ZK Slashing Evidence System

## Overview

The ZK (Zero-Knowledge) Slashing Evidence system provides **automated cryptographic proof** that a validator committed a slashable offense, without requiring manual validator voting or revealing sensitive information.

## The Problem

Traditional slashing requires:
1. Someone submits evidence
2. Validators manually vote on validity
3. 3+ validators must agree
4. Slow (3-day voting window)
5. Subject to voter apathy

## The Solution: ZK Proofs

With ZK proofs:
1. Anyone generates a cryptographic proof
2. Smart contract **automatically verifies** proof
3. Slashing executes immediately if valid
4. No voting required
5. Mathematically certain

## How It Works

### Double-Signing Scenario

```
Validator Alice votes:
  Time T=0: "APPROVE package P at block 100"
  Time T=1: "REJECT package P at block 100"

This is a SLASHABLE offense (double-signing)

┌─────────────────────────────────────────────────────────────┐
│                    ZK PROOF GENERATION                       │
├─────────────────────────────────────────────────────────────┤
│                                                              │
│  Public Inputs (Visible to all):                            │
│  ├── Alice's Public Key (PK_A)                              │
│  ├── Package Hash: H(P)                                     │
│  ├── Vote 1 Hash: H("APPROVE:P:100")                        │
│  └── Vote 2 Hash: H("REJECT:P:100")                         │
│                                                              │
│  Private Inputs (Kept secret):                              │
│  ├── Alice's Private Key (kept hidden!)                     │
│  ├── Signature 1 on Vote 1                                  │
│  └── Signature 2 on Vote 2                                  │
│                                                              │
│  Circuit Proves:                                            │
│  1. Both signatures valid under PK_A                        │
│  2. Signatures on DIFFERENT votes                           │
│  3. Private key was known (can't forge)                     │
│                                                              │
│  Output: Zero-Knowledge Proof π                             │
│                                                              │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│                 ON-CHAIN VERIFICATION                        │
├─────────────────────────────────────────────────────────────┤
│                                                              │
│  Anyone submits: (π, Public Inputs)                         │
│                                                              │
│  Smart Contract:                                             │
│  1. Verify π is valid Groth16 proof                         │
│  2. Check Public Inputs match Alice                         │
│  3. Check π not used before (nullifier)                     │
│                                                              │
│  If all checks pass:                                        │
│  ├── ✅ Proof verified                                      │
│  ├── ✅ Alice slashed immediately                           │
│  └── ✅ Whistleblower rewarded                              │
│                                                              │
│  No validator voting needed!                                │
│                                                              │
└─────────────────────────────────────────────────────────────┘
```

## Technical Architecture

### 1. Circom Circuit

**File:** `circuits/DoubleSignProof.circom`

```circom
Public Inputs:
- validatorPubkey[2]: Ed25519 public key (x, y)
- packageHash: Poseidon hash of package
- vote1Hash: Hash of first vote
- vote2Hash: Hash of second vote

Private Inputs:
- validatorPrivkey: Private key (kept secret!)
- signature1[3]: EdDSA signature (R_x, R_y, S)
- signature2[3]: EdDSA signature (R_x, R_y, S)

Constraints:
1. vote1Hash != vote2Hash (conflicting votes)
2. Verify signature1 with validatorPubkey
3. Verify signature2 with validatorPubkey
```

**Why EdDSA/Poseidon?**
- EdDSA: Standard for blockchain signatures
- Poseidon: ZK-friendly hash function (fewer constraints)
- Groth16: Efficient proof generation/verification

### 2. Solidity Verifier

**File:** `contracts/ZKSlashingVerifier.sol`

```solidity
function verifyDoubleSign(
    Proof calldata proof,
    uint256[5] calldata publicInputs
) external returns (bool valid, bytes32 nullifier) {
    // Verify Groth16 proof
    require(_verifyProof(vk, proof, publicInputs), "Invalid proof");
    
    // Prevent replay
    bytes32 nullifier = keccak256(abi.encodePacked(publicInputs));
    require(!usedNullifiers[nullifier], "Already used");
    usedNullifiers[nullifier] = true;
    
    return (true, nullifier);
}
```

### 3. Rust Proof Generator

**File:** `crates/zk-validator/src/slashing.rs`

```rust
// Generate proof
let generator = SlashingProofGenerator::new(config);
let proof = generator.generate_double_sign_proof(&evidence).await?;

// Monitor for double-signs
let mut monitor = DoubleSignMonitor::new(generator);
if let Some(evidence) = monitor.record_vote(vote) {
    // Double-sign detected!
    submit_proof_to_chain(proof).await?;
}
```

## Security Properties

| Property | Guarantee |
|----------|-----------|
| **Soundness** | Invalid proofs rejected |
| **Completeness** | Valid proofs accepted |
| **Zero-Knowledge** | Private key not revealed |
| **Non-Replay** | Each proof has unique nullifier |
| **Permissionless** | Anyone can submit proof |

## Comparison: Traditional vs ZK Slashing

| Feature | Traditional | ZK Slashing |
|---------|-------------|-------------|
| Voting | 3+ validators vote | Automatic verification |
| Time | 3 days | Instant |
| Cost | Gas for voting | Gas for one verification |
| Certainty | Subjective | Mathematical |
| Whistleblower | Must convince validators | Just submit proof |
| Validator Work | Review evidence | None |

## Usage

### Generating a Proof

```bash
# Detect and prove double-signing
creg slashing detect-double-sign \
  --validator 0xAlice \
  --package "npm:malicious@1.0.0" \
  --vote1-approve \
  --vote2-reject \
  --output proof.json

# Submit to chain
creg slashing submit-proof proof.json
```

### Programmatic Usage

```rust
use zk_validator::slashing::*;

// Create evidence
let evidence = DoubleSignEvidence {
    public_inputs: DoubleSignPublicInputs {
        validator_pubkey_x: "0x1234...".to_string(),
        validator_pubkey_y: "0x5678...".to_string(),
        package_hash: "0xabcd...".to_string(),
        vote1_hash: "0xdef0...".to_string(),
        vote2_hash: "0x1234...".to_string(),
    },
    witness: DoubleSignWitness {
        // Private inputs
        validator_privkey: "0xdeadbeef...".to_string(),
        signature1: Signature { ... },
        signature2: Signature { ... },
    },
    // ...
};

// Generate proof
let generator = SlashingProofGenerator::new(config);
let proof = generator.generate_double_sign_proof(&evidence).await?;

// Submit to contract
contract.submitZKDoubleSignEvidence(proof.proof, proof.public_inputs, offender).await?;
```

## Economic Incentives

### Costs

| Operation | Cost |
|-----------|------|
| Proof Generation | ~10 seconds CPU + 100MB RAM |
| Gas (verification) | ~200,000 gas |

### Rewards

| Role | Reward |
|------|--------|
| Proof Generator (Whistleblower) | 10% of slashed amount |
| Protocol | 90% of slashed amount (burned or redistributed) |

## Circuit Constraints

```
DoubleSignProof Circuit:
├── EdDSA Verification 1: ~10,000 constraints
├── EdDSA Verification 2: ~10,000 constraints
├── Hash Equality Check: ~100 constraints
└── Total: ~20,100 constraints

Proof Generation Time: ~10 seconds (consumer CPU)
Proof Verification Gas: ~200,000 (on Ethereum)
Proof Size: ~200 bytes
```

## Future Proof Types

### 1. False Approval
Prove validator approved package that was later revoked as malicious.

```circom
Public: validator_pubkey, package_hash, approval_timestamp
Private: approval_signature, revocation_evidence
Prove: approval came before revocation
```

### 2. Griefing
Prove validator consistently votes against majority without justification.

```circom
Public: validator_pubkey, vote_history_hash
Private: individual votes
Prove: >90% disagreement with majority over 100 votes
```

## Integration with Existing Contracts

```solidity
// Existing SlashingEvidence contract
contract SlashingEvidence {
    function submitEvidence(...) external;
}

// New ZK-enabled extension
contract ZKEnabledSlashing is SlashingEvidence {
    ZKSlashingVerifier public verifier;
    
    function submitZKDoubleSignEvidence(
        Proof calldata proof,
        uint256[5] calldata publicInputs,
        address offender
    ) external {
        // Verify ZK proof
        (bool valid, bytes32 nullifier) = verifier.verifyDoubleSign(
            proof, 
            publicInputs
        );
        require(valid, "Invalid proof");
        
        // Execute slashing immediately
        _slash(offender, SLASH_AMOUNT_DOUBLE_SIGN);
        
        // Reward whistleblower
        _reward(msg.sender, SLASH_AMOUNT_DOUBLE_SIGN * 10 / 100);
    }
}
```

## Deployment

### 1. Compile Circuit

```bash
cd circuits

circom DoubleSignProof.circom --r1cs --wasm --sym

# Generate proving key
snarkjs groth16 setup DoubleSignProof.r1cs pot12_final.ptau DoubleSignProof_0000.zkey
snarkjs zkey contribute DoubleSignProof_0000.zkey DoubleSignProof_final.zkey --name="Contributer 1"

# Export verifier
snarkjs zkey export solidityverifier DoubleSignProof_final.zkey verifier.sol
```

### 2. Deploy Contracts

```bash
forge script script/DeployZKSlashing.s.sol --rpc-url $RPC_URL --broadcast
```

### 3. Configure

```solidity
// Set verifying key
ZKSlashingVerifier.setVerifyingKey(
    PROOF_TYPE_DOUBLE_SIGN,
    loadVerifyingKey("DoubleSignProof_vk.json")
);
```

## Testing

```bash
# Run unit tests
cargo test -p zk-validator

# Run circuit tests
cd circuits && npm test

# Integration test
forge test --match-contract ZKSlashingTest
```

## Security Considerations

### 1. Trusted Setup
Groth16 requires trusted setup. Use:
- Multi-party computation (MPC) ceremony
- Or transparent alternatives (PLONK, STARKs)

### 2. Circuit Audits
- Verify no backdoors in circuit
- Check constraint soundness
- Formal verification recommended

### 3. Front-Running
Anyone can submit valid proofs. Use:
- Commit-reveal scheme
- Or accept first valid proof

## Troubleshooting

### "Invalid proof"
- Check public inputs match
- Verify proving key version
- Ensure circuit hasn't changed

### "Proof already used"
- Each evidence can only be submitted once
- Nullifier prevents replay
- Check if someone else submitted first

### "Verification gas too high"
- Use batch verification
- Or verify on L2 (Arbitrum/Optimism)
- Or use recursive proofs

## Resources

- **Circuit**: `circuits/DoubleSignProof.circom`
- **Verifier**: `contracts/ZKSlashingVerifier.sol`
- **Proof Gen**: `crates/zk-validator/src/slashing.rs`
- **Tests**: `forge test --match-contract ZKSlashing`

## References

- [Groth16 Paper](https://eprint.iacr.org/2016/260)
- [Circom Documentation](https://docs.circom.io/)
- [EdDSA Signatures](https://ed25519.cr.yp.to/)
- [Poseidon Hash](https://eprint.iacr.org/2019/458)

---

*Last updated: 2026-04-02*
