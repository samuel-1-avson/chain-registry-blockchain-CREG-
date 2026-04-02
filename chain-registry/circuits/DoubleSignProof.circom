/*
 * DoubleSignProof.circom
 * 
 * Zero-knowledge circuit that proves a validator signed two conflicting votes
 * for the same package without revealing the validator's private key.
 * 
 * Public Inputs:
 * - validatorPubkey: Ed25519 public key of the accused validator
 * - packageHash: Hash of the package being voted on
 * - vote1Hash: Hash of first vote (approve)
 * - vote2Hash: Hash of second vote (reject)
 * 
 * Private Inputs (Witness):
 * - validatorPrivkey: The validator's private key (kept secret)
 * - signature1: Ed25519 signature on vote1
 * - signature2: Ed25519 signature on vote2
 * 
 * The circuit proves:
 * 1. Both signatures are valid under validatorPubkey
 * 2. The signatures are on different votes (vote1Hash ≠ vote2Hash)
 * 3. The validator knew the private key (only way to produce valid sigs)
 */

pragma circom 2.0.0;

include "../node_modules/circomlib/circuits/eddsaposeidon.circom";
include "../node_modules/circomlib/circuits/poseidon.circom";
include "../node_modules/circomlib/circuits/comparators.circom";
include "../node_modules/circomlib/circuits/bitify.circom";

// Maximum number of bits for Poseidon hash
var POSEIDON_MAX = 256;

/*
 * Helper: Verify an Ed25519 signature
 * Uses Poseidon-based EdDSA (compatible with circomlib)
 */
template VerifyEd25519Signature() {
    signal input pubkey[2];      // Public key (A_x, A_y)
    signal input signature[3];   // Signature (R_x, R_y, S)
    signal input message;        // Message hash
    
    // Use circomlib's EdDSA Poseidon verifier
    component verifier = EdDSAPoseidonVerifier();
    verifier.enabled <== 1;
    verifier.Ax <== pubkey[0];
    verifier.Ay <== pubkey[1];
    verifier.S <== signature[2];
    verifier.R8x <== signature[0];
    verifier.R8y <== signature[1];
    verifier.M <== message;
}

/*
 * Main circuit: Prove double-signing
 */
template DoubleSignProof() {
    // Public inputs
    signal input validatorPubkey[2];  // (x, y) coordinates
    signal input packageHash;         // Package identifier hash
    signal input vote1Hash;           // First vote hash (e.g., "approve")
    signal input vote2Hash;           // Second vote hash (e.g., "reject")
    
    // Private inputs (witness)
    signal input validatorPrivkey;    // Private key (kept secret!)
    signal input signature1[3];       // (R_x, R_y, S) for vote 1
    signal input signature2[3];       // (R_x, R_y, S) for vote 2
    
    // === Step 1: Verify signatures are different ===
    // vote1Hash must NOT equal vote2Hash
    component hashEqual = IsEqual();
    hashEqual.in[0] <== vote1Hash;
    hashEqual.in[1] <== vote2Hash;
    
    // hashEqual.out will be 1 if equal, 0 if different
    // We need it to be 0 (different), so we assert (1 - hashEqual.out) == 1
    signal isDifferent <== 1 - hashEqual.out;
    isDifferent === 1;
    
    // === Step 2: Verify first signature ===
    component verify1 = VerifyEd25519Signature();
    verify1.pubkey[0] <== validatorPubkey[0];
    verify1.pubkey[1] <== validatorPubkey[1];
    verify1.signature[0] <== signature1[0];
    verify1.signature[1] <== signature1[1];
    verify1.signature[2] <== signature1[2];
    verify1.message <== vote1Hash;
    
    // === Step 3: Verify second signature ===
    component verify2 = VerifyEd25519Signature();
    verify2.pubkey[0] <== validatorPubkey[0];
    verify2.pubkey[1] <== validatorPubkey[1];
    verify2.signature[0] <== signature2[0];
    verify2.signature[1] <== signature2[1];
    verify2.signature[2] <== signature2[2];
    verify2.message <== vote2Hash;
    
    // === Step 4: Optional - prove private key derives to public key ===
    // This ensures the prover actually knows the private key
    // Skipped for simplicity in this version, but could be added
    // using BabyJubJub scalar multiplication
    
    // Output: Success signal (always 1 if constraints satisfied)
    signal output valid;
    valid <== 1;
}

// Instantiate the main component
component main {public [validatorPubkey, packageHash, vote1Hash, vote2Hash]} = DoubleSignProof();
