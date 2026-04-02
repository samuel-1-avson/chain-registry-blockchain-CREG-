# ZK Circuits for Chain Registry

This directory contains zero-knowledge circuits for the Chain Registry protocol.

## Circuits

### DoubleSignProof.circom

Proves that a validator signed two conflicting votes for the same package.

**Public Inputs:**
- `validatorPubkey[2]`: Ed25519 public key (x, y coordinates)
- `packageHash`: Poseidon hash of package canonical
- `vote1Hash`: Hash of first vote (approve)
- `vote2Hash`: Hash of second vote (reject)

**Private Inputs:**
- `validatorPrivkey`: Private key (kept secret!)
- `signature1[3]`: EdDSA signature (R_x, R_y, S)
- `signature2[3]`: EdDSA signature (R_x, R_y, S)

**Constraints:**
1. Both signatures are valid under the public key
2. The votes are different (one approve, one reject)
3. Both votes are for the same package

## Building

### Prerequisites

1. Install Circom:
```bash
curl --proto '=https' --tlsv1.2 https://sh.rustup.rs -sSf | sh
git clone https://github.com/iden3/circom.git
cd circom
cargo build --release
cargo install --path circom
```

2. Install snarkjs:
```bash
npm install -g snarkjs
```

3. Install dependencies:
```bash
npm install
```

### Build Circuit

```bash
chmod +x build_circuit.sh
./build_circuit.sh
```

This will:
1. Compile the circuit to R1CS
2. Run trusted setup (powers of tau)
3. Generate proving and verification keys
4. Export Solidity verifier contract

## Testing

```bash
# Generate test proof
cd build/DoubleSignProof_js
node generate_witness.js DoubleSignProof.wasm input.json witness.wtns

# Create proof
snarkjs groth16 prove ../../keys/DoubleSignProof_final.zkey witness.wtns proof.json public.json

# Verify proof
snarkjs groth16 verify ../../keys/verification_key.json public.json proof.json
```

## Circuit Constraints

| Component | Constraints |
|-----------|-------------|
| EdDSA Verify (2x) | ~20,000 |
| Hash Comparison | ~100 |
| Range Checks | ~500 |
| **Total** | **~20,600** |

## Gas Costs

| Operation | Gas |
|-----------|-----|
| Proof Verification | ~200,000 |
| Submit Evidence (incl. verification) | ~250,000 |

## Security

- **Soundness**: Invalid proofs rejected with overwhelming probability
- **Zero-Knowledge**: Private inputs not revealed
- **Non-Replay**: Nullifier system prevents double-spending proofs

## Files

- `DoubleSignProof.circom` - Main circuit definition
- `ed25519_verify.circom` - EdDSA signature verification gadget
- `poseidon_hasher.circom` - Poseidon hash function
- `build/` - Build artifacts (R1CS, WASM, witness gen)
- `keys/` - Proving and verification keys

## References

- [Circom Documentation](https://docs.circom.io/)
- [Circomlib](https://github.com/iden3/circomlib)
- [Groth16 Paper](https://eprint.iacr.org/2016/260)
- [EdDSA Signatures](https://ed25519.cr.yp.to/)
- [Poseidon Hash](https://eprint.iacr.org/2019/458)
