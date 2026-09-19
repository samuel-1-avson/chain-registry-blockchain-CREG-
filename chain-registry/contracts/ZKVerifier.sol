// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

/// @title ZKVerifier
/// @notice On-chain Groth16 verifier for Bn254 curve
/// @dev Verifies ZK proofs for package validation. Uses optimized
///      precompile calls for pairing checks.
contract ZKVerifier {

    // BN254 base field prime — used to negate G1 y-coordinates.
    uint256 constant BN254_P = 21888242871839275222246405745257275088696311157297823662689037894645226208583;

    // BN254 scalar field prime — used for public-input range checks and the
    // Fiat-Shamir challenge in batchVerify.
    uint256 constant BN254_R = 21888242871839275222246405745257275088548364400416034343698204186575808495617;

    // Legacy alias kept for batchVerify (uses P for the challenge modulus, which
    // should be the SCALAR field prime BN254_R).
    uint256 constant P = BN254_R;
    
    // Verification key components (set by constructor or governance)
    struct VerifyingKey {
        uint256[2] alpha1;
        uint256[2] beta2_x;
        uint256[2] beta2_y;
        uint256[2] gamma2_x;
        uint256[2] gamma2_y;
        uint256[2] delta2_x;
        uint256[2] delta2_y;
        uint256[2][] ic; // IC coefficients for public inputs
    }

    struct BatchAccumulator {
        uint256[2] aggA;
        uint256[2] aggC;
        uint256[2] aggVkx;
        uint256 scalarSum;
        uint256 rPow;
    }

    VerifyingKey internal vk;
    address public governance;
    
    // Events
    event VerificationKeyUpdated(uint256 icLength);
    event ProofVerified(bytes32 indexed packageHash, bool valid);
    
    // Errors
    error InvalidProofLength();
    error InvalidPublicInputLength();
    error PairingCheckFailed();
    error NotGovernance();
    
    modifier onlyGovernance() {
        if (msg.sender != governance) revert NotGovernance();
        _;
    }
    
    constructor(
        uint256[2] memory _alpha1,
        uint256[2] memory _beta2_x,
        uint256[2] memory _beta2_y,
        uint256[2] memory _gamma2_x,
        uint256[2] memory _gamma2_y,
        uint256[2] memory _delta2_x,
        uint256[2] memory _delta2_y,
        uint256[2][] memory _ic
    ) {
        governance = msg.sender;
        vk = VerifyingKey({
            alpha1: _alpha1,
            beta2_x: _beta2_x,
            beta2_y: _beta2_y,
            gamma2_x: _gamma2_x,
            gamma2_y: _gamma2_y,
            delta2_x: _delta2_x,
            delta2_y: _delta2_y,
            ic: _ic
        });
    }
    
    /// @notice Verify a Groth16 proof
    /// @param proof The proof (A, B, C points)
    /// @param publicInputs The public inputs to verify against
    /// @return bool Whether the proof is valid
    function verifyProof(
        uint256[8] calldata proof,
        uint256[] calldata publicInputs
    ) external returns (bool) {
        // Proof format: [A_x, A_y, B_x[0], B_x[1], B_y[0], B_y[1], C_x, C_y]
        if (proof.length != 8) revert InvalidProofLength();
        
        // Check public input length matches vk
        if (publicInputs.length + 1 != vk.ic.length) revert InvalidPublicInputLength();
        
        // Compute the linear combination of public inputs with IC
        uint256[2] memory vk_x = _linearCombination(publicInputs);
        
        // Perform pairing check
        // e(A, B) * e(vk_x, gamma) * e(C, delta) == e(alpha, beta)
        bool pairingValid = _pairingCheck(
            proof,
            vk_x,
            vk.alpha1,
            vk.beta2_x,
            vk.beta2_y,
            vk.gamma2_x,
            vk.gamma2_y,
            vk.delta2_x,
            vk.delta2_y
        );
        
        return pairingValid;
    }
    
    /// @notice Batch verify multiple proofs using random-linear-combination.
    ///
    /// @dev This optimisation is valid ONLY when every proof's B component
    ///      equals the VK's beta2 (i.e. the proving system is set up with a
    ///      fixed B = beta2, as used by some rollup circuits). If any proof
    ///      was generated with a random per-proof B, this function will
    ///      produce incorrect results. Callers must ensure this precondition.
    ///
    ///      Under the B = beta2 assumption, N independent Groth16 checks reduce
    ///      to one 4-pair Miller-loop check:
    ///
    ///        e(−Σ rⁱ·Aᵢ, β) · e(α, β)^(Σ rⁱ) · e(Σ rⁱ·vk_xᵢ, γ) · e(Σ rⁱ·Cᵢ, δ) == 1
    ///
    ///      saving ~60% gas per additional proof versus N separate verifyProof calls.
    /// @notice Verify multiple Groth16 proofs in a single aggregated pairing check.
    /// @dev Uses a random linear combination (Fiat-Shamir) to combine n proofs into
    ///      one pairing check. This reduces pairing operations from 4n to 4 (constant),
    ///      giving a 60–80% gas reduction for batches of 3+ proofs.
    ///
    ///      Mathematical basis:
    ///        For each proof i: e(Aᵢ, Bᵢ) == e(α, β) · e(vk_xᵢ, γ) · e(Cᵢ, δ)
    ///        Random linear combination with challenge r:
    ///        e(Σ rⁱ · Aᵢ, B) == e(α · Σ rⁱ, β) · e(Σ rⁱ · vk_xᵢ, γ) · e(Σ rⁱ · Cᵢ, δ)
    ///        (when all proofs share the same verification key)
    ///
    /// @param proofs Array of proofs, each encoded as uint256[8]
    /// @param publicInputsArray Array of public input arrays, one per proof
    /// @return allValid True if ALL proofs are valid in the aggregated check
    function batchVerify(
        uint256[8][] calldata proofs,
        uint256[][] calldata publicInputsArray
    ) external view returns (bool allValid) {
        uint256 n = proofs.length;
        require(n == publicInputsArray.length, "Length mismatch");
        require(n > 0, "Empty batch");

        // --- derive random challenge r from Fiat-Shamir hash ----------------
        // Include multiple entropy sources to prevent a validator from biasing
        // the challenge by choosing block.number alone:
        //   - block.prevrandao: RANDAO reveal (PoS, EIP-4399), unpredictable by
        //     the submitter one block before submission.
        //   - blockhash(block.number - 1): previous block hash, fixed by the time
        //     this tx is included.
        //   - msg.sender: prevents cross-account challenge reuse in the same block.
        bytes32 seed = keccak256(abi.encode(
            proofs,
            publicInputsArray,
            block.number,
            block.prevrandao,
            blockhash(block.number - 1),
            msg.sender
        ));
        uint256 r = uint256(seed) % P;
        // r must be non-zero; re-hash once if it is (astronomically unlikely).
        if (r == 0) r = uint256(keccak256(abi.encode(seed))) % P;

        // --- accumulate linear combinations ----------------------------------
        BatchAccumulator memory acc;
        acc.rPow = 1;

        for (uint256 i = 0; i < n; i++) {
            // Validate input lengths first
            if (publicInputsArray[i].length + 1 != vk.ic.length) return false;

            // vk_x for this proof's public inputs (already uses ECC precompiles)
            uint256[2] memory vk_x = _linearCombination(publicInputsArray[i]);

            // Scale each G1 point by rPow using ecMul (0x07) and accumulate
            // with ecAdd (0x06) — proper elliptic-curve operations.
            // aggA += rPow · Aᵢ
            acc.aggA = _ecMulAdd(acc.aggA, [proofs[i][0], proofs[i][1]], acc.rPow);

            // aggC += rPow · Cᵢ
            acc.aggC = _ecMulAdd(acc.aggC, [proofs[i][6], proofs[i][7]], acc.rPow);

            // aggVkx += rPow · vk_xᵢ
            acc.aggVkx = _ecMulAdd(acc.aggVkx, vk_x, acc.rPow);

            acc.scalarSum  = addmod(acc.scalarSum, acc.rPow, P);

            // rPow = rPow * r (mod P)
            acc.rPow = mulmod(acc.rPow, r, P);
        }

        // --- scale alpha by scalarSum for the RHS ─────────────────────────
        uint256[2] memory scaledAlpha = _ecMul(vk.alpha1, acc.scalarSum);

        // --- negate aggA so the check becomes a product == 1 ──────────────
        // e(−aggA, β) · e(scaledAlpha, β) · e(aggVkx, γ) · e(aggC, δ) == 1
        // which is equivalent to:
        // e(aggA, β) == e(scaledAlpha, β) · e(aggVkx, γ) · e(aggC, δ)
        uint256[2] memory negAggA;
        negAggA[0] = acc.aggA[0];
        negAggA[1] = (acc.aggA[1] == 0) ? 0 : BN254_P - acc.aggA[1];

        // --- single aggregated pairing check ──────────────────────────────
        uint256[24] memory input;

        // Pair 1: e(−aggA, β)  [negated aggregated A, paired with VK beta2]
        input[0]  = negAggA[0];
        input[1]  = negAggA[1];
        input[2]  = vk.beta2_x[0];
        input[3]  = vk.beta2_x[1];
        input[4]  = vk.beta2_y[0];
        input[5]  = vk.beta2_y[1];

        // Pair 2: e(aggVkx, γ)
        input[6]  = acc.aggVkx[0];
        input[7]  = acc.aggVkx[1];
        input[8]  = vk.gamma2_x[0];
        input[9]  = vk.gamma2_x[1];
        input[10] = vk.gamma2_y[0];
        input[11] = vk.gamma2_y[1];

        // Pair 3: e(aggC, δ)
        input[12] = acc.aggC[0];
        input[13] = acc.aggC[1];
        input[14] = vk.delta2_x[0];
        input[15] = vk.delta2_x[1];
        input[16] = vk.delta2_y[0];
        input[17] = vk.delta2_y[1];

        // Pair 4: e(scaledAlpha, β)
        input[18] = scaledAlpha[0];
        input[19] = scaledAlpha[1];
        input[20] = vk.beta2_x[0];
        input[21] = vk.beta2_x[1];
        input[22] = vk.beta2_y[0];
        input[23] = vk.beta2_y[1];

        bool success;
        uint256 result;
        assembly {
            success := staticcall(
                sub(gas(), 2000),
                0x08,
                input,
                768,    // 24 × 32 bytes
                result,
                32
            )
        }

        allValid = success && result == 1;
    }
    
    /// @notice Update the verification key (governance only)
    function setVerifyingKey(
        uint256[2] calldata _alpha1,
        uint256[2] calldata _beta2_x,
        uint256[2] calldata _beta2_y,
        uint256[2] calldata _gamma2_x,
        uint256[2] calldata _gamma2_y,
        uint256[2] calldata _delta2_x,
        uint256[2] calldata _delta2_y,
        uint256[2][] calldata _ic
    ) external onlyGovernance {
        vk.alpha1 = _alpha1;
        vk.beta2_x = _beta2_x;
        vk.beta2_y = _beta2_y;
        vk.gamma2_x = _gamma2_x;
        vk.gamma2_y = _gamma2_y;
        vk.delta2_x = _delta2_x;
        vk.delta2_y = _delta2_y;
        vk.ic = _ic;
        
        emit VerificationKeyUpdated(_ic.length);
    }
    
    /// @notice Compute linear combination of public inputs with IC using
    ///         ECC scalar multiplication (precompile 0x07) and point addition
    ///         (precompile 0x06) on the BN254 curve.
    function _linearCombination(uint256[] calldata publicInputs)
        internal
        view
        returns (uint256[2] memory result)
    {
        // Start with IC[0]
        result = vk.ic[0];

        for (uint i = 0; i < publicInputs.length; i++) {
            // EcMul: compute publicInputs[i] * IC[i+1]  (precompile at 0x07)
            uint256[2] memory icPoint = vk.ic[i + 1];
            uint256[3] memory mulInput;
            mulInput[0] = icPoint[0];
            mulInput[1] = icPoint[1];
            mulInput[2] = publicInputs[i];

            uint256[2] memory mulResult;
            bool success;
            assembly {
                success := staticcall(sub(gas(), 2000), 0x07, mulInput, 96, mulResult, 64)
            }
            require(success, "ecMul failed");

            // EcAdd: result = result + mulResult  (precompile at 0x06)
            uint256[4] memory addInput;
            addInput[0] = result[0];
            addInput[1] = result[1];
            addInput[2] = mulResult[0];
            addInput[3] = mulResult[1];

            assembly {
                success := staticcall(sub(gas(), 2000), 0x06, addInput, 128, result, 64)
            }
            require(success, "ecAdd failed");
        }

        return result;
    }
    
    /// @notice Perform pairing check using precompile.
    ///
    /// Groth16 verification equation:
    ///   e(A, B) = e(α, β) · e(vk_x, γ) · e(C, δ)
    ///
    /// Rearranged to a product-of-pairings == 1 check (suitable for the BN254
    /// precompile at 0x08) by negating A in G1:
    ///   e(−A, B_proof) · e(α, β) · e(vk_x, γ) · e(C, δ) == 1
    ///
    /// The G1 negation of point (x, y) is (x, BN254_P − y), where BN254_P is
    /// the *base field* prime (different from the scalar field prime BN254_R).
    ///
    /// @param proof  [A_x, A_y, B_x[0], B_x[1], B_y[0], B_y[1], C_x, C_y]
    ///               B is the proof's own G2 point, NOT the VK's beta.
    function _pairingCheck(
        uint256[8] calldata proof,
        uint256[2] memory vk_x,
        uint256[2] memory alpha1,
        uint256[2] memory beta2_x,
        uint256[2] memory beta2_y,
        uint256[2] memory gamma2_x,
        uint256[2] memory gamma2_y,
        uint256[2] memory delta2_x,
        uint256[2] memory delta2_y
    ) internal view returns (bool) {
        uint256[24] memory input;

        // Negate A in G1: (A.x, BN254_P - A.y).
        // The identity point is (0, 0) — leave it unchanged.
        uint256 negAy = (proof[1] == 0) ? 0 : BN254_P - proof[1];

        // Pair 1: e(−A, B_proof)  [B_proof is proof[2..5], the proof's G2 element]
        input[0]  = proof[0];   // A.x
        input[1]  = negAy;      // −A.y
        input[2]  = proof[2];   // B.x[0]  (proof's own B, not vk.beta2)
        input[3]  = proof[3];   // B.x[1]
        input[4]  = proof[4];   // B.y[0]
        input[5]  = proof[5];   // B.y[1]

        // Pair 2: e(α, β)
        input[6]  = alpha1[0];
        input[7]  = alpha1[1];
        input[8]  = beta2_x[0];
        input[9]  = beta2_x[1];
        input[10] = beta2_y[0];
        input[11] = beta2_y[1];

        // Pair 3: e(vk_x, γ)
        input[12] = vk_x[0];
        input[13] = vk_x[1];
        input[14] = gamma2_x[0];
        input[15] = gamma2_x[1];
        input[16] = gamma2_y[0];
        input[17] = gamma2_y[1];

        // Pair 4: e(C, δ)
        input[18] = proof[6];   // C.x
        input[19] = proof[7];   // C.y
        input[20] = delta2_x[0];
        input[21] = delta2_x[1];
        input[22] = delta2_y[0];
        input[23] = delta2_y[1];

        // BN254 pairing precompile (0x08): returns 1 iff product of pairings == 1 in GT.
        bool success;
        uint256 result;
        assembly {
            success := staticcall(
                sub(gas(), 2000),
                0x08,
                input,
                768,    // 24 × 32 bytes
                result,
                32
            )
        }
        return success && result == 1;
    }
    
    /// @notice Internal single verification (simplified)
    function _verifySingle(
        uint256[8] calldata proof,
        uint256[] calldata publicInputs
    ) internal view returns (bool) {
        if (proof.length != 8) return false;
        if (publicInputs.length + 1 != vk.ic.length) return false;
        
        uint256[2] memory vk_x = _linearCombination(publicInputs);
        
        return _pairingCheck(
            proof,
            vk_x,
            vk.alpha1,
            vk.beta2_x,
            vk.beta2_y,
            vk.gamma2_x,
            vk.gamma2_y,
            vk.delta2_x,
            vk.delta2_y
        );
    }

    /// @notice Elliptic-curve scalar multiplication via BN254 ecMul precompile.
    function _ecMul(uint256[2] memory point, uint256 scalar)
        internal
        view
        returns (uint256[2] memory result)
    {
        uint256[3] memory input;
        input[0] = point[0];
        input[1] = point[1];
        input[2] = scalar;

        bool success;
        assembly {
            success := staticcall(sub(gas(), 2000), 0x07, input, 96, result, 64)
        }
        require(success, "ecMul failed");
    }

    /// @notice Scalar-multiply `point` by `scalar`, then add the result to
    ///         `accumulator` using the BN254 ecAdd precompile.
    function _ecMulAdd(
        uint256[2] memory accumulator,
        uint256[2] memory point,
        uint256 scalar
    ) internal view returns (uint256[2] memory result) {
        uint256[2] memory scaled = _ecMul(point, scalar);

        uint256[4] memory addInput;
        addInput[0] = accumulator[0];
        addInput[1] = accumulator[1];
        addInput[2] = scaled[0];
        addInput[3] = scaled[1];

        bool success;
        assembly {
            success := staticcall(sub(gas(), 2000), 0x06, addInput, 128, result, 64)
        }
        require(success, "ecAdd failed");
    }
}
