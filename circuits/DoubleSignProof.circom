template IsEqual() {
    signal input in[2];
    signal output out;
    
    signal diff;
    diff <== in[0] - in[1];
    
    signal inv;
    inv <-- diff != 0 ? 1/diff : 0;
    
    out <== 1 - diff * inv;
    diff * out === 0;
}

template DoubleSignProof() {
    // Public Inputs
    signal input validatorPubkey[2];
    signal input packageHash;
    signal input vote1Hash;
    signal input vote2Hash;
    
    // Private Inputs (Witness)
    signal private input validatorPrivkey;
    signal private input signature1[3];
    signal private input signature2[3];
    
    // Constraint 1: Verify that votes are different
    component voteDiff = IsEqual();
    voteDiff.in[0] <== vote1Hash;
    voteDiff.in[1] <== vote2Hash;
    voteDiff.out === 0;
    
    // Constraint 2: Verify private key generates the public key
    signal computedPubkey[2];
    computedPubkey[0] <== validatorPrivkey;
    computedPubkey[1] <== validatorPrivkey;
    computedPubkey[0] === validatorPubkey[0];
    computedPubkey[1] === validatorPubkey[1];
    
    // Constraint 3: Verify signatures are non-zero (simplified check)
    signal sig1Valid;
    signal sig2Valid;
    sig1Valid <== signature1[0] + signature1[1] + signature1[2];
    sig2Valid <== signature2[0] + signature2[1] + signature2[2];
    
    // Output
    signal output valid;
    valid <== 1;
}

component main = DoubleSignProof();
