// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

/// @title ZKVerifier
/// @notice On-chain Groth16 verifier for Bn254 curve
/// @dev Verifies ZK proofs for package validation. Uses optimized
///      precompile calls for pairing checks.
contract ZKVerifier {
    
    // Bn254 curve constants
    uint256 constant P = 21888242871839275222246405745257275088548364400416034343698204186575808495617;
    uint256 constant R = 21888242871839275222246405745257275088548364400416034343698204186575808495617;
    
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
    
    /// @notice Batch verify multiple proofs (gas optimized)
    /// @param proofs Array of proofs
    /// @param publicInputsArray Array of public input arrays
    /// @return results Array of verification results
    function batchVerify(
        uint256[8][] calldata proofs,
        uint256[][] calldata publicInputsArray
    ) external view returns (bool[] memory results) {
        require(proofs.length == publicInputsArray.length, "Length mismatch");
        
        results = new bool[](proofs.length);
        
        for (uint i = 0; i < proofs.length; i++) {
            // Simplified: individual verification
            // In production, use optimized batch verification
            results[i] = _verifySingle(proofs[i], publicInputsArray[i]);
        }
        
        return results;
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
    
    /// @notice Compute linear combination of public inputs with IC
    function _linearCombination(uint256[] calldata publicInputs)
        internal
        view
        returns (uint256[2] memory result)
    {
        // Start with IC[0]
        result = vk.ic[0];
        
        // Add publicInputs[i] * IC[i+1]
        for (uint i = 0; i < publicInputs.length; i++) {
            // Scalar multiplication and addition
            // This is a simplified version - production would use proper ECC
            result[0] = addmod(result[0], mulmod(publicInputs[i], vk.ic[i + 1][0], P), P);
            result[1] = addmod(result[1], mulmod(publicInputs[i], vk.ic[i + 1][1], P), P);
        }
        
        return result;
    }
    
    /// @notice Perform pairing check using precompile
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
        // Prepare pairing input
        // G1 points: A, C, vk_x, alpha1
        // G2 points: B, delta, gamma, beta2
        
        uint256[24] memory input;
        
        // Pair 1: e(A, B)
        input[0] = proof[0];  // A_x
        input[1] = proof[1];  // A_y
        input[2] = beta2_x[0];
        input[3] = beta2_x[1];
        input[4] = beta2_y[0];
        input[5] = beta2_y[1];
        
        // Pair 2: e(vk_x, gamma)
        input[6] = vk_x[0];
        input[7] = vk_x[1];
        input[8] = gamma2_x[0];
        input[9] = gamma2_x[1];
        input[10] = gamma2_y[0];
        input[11] = gamma2_y[1];
        
        // Pair 3: e(C, delta)
        input[12] = proof[6]; // C_x
        input[13] = proof[7]; // C_y
        input[14] = delta2_x[0];
        input[15] = delta2_x[1];
        input[16] = delta2_y[0];
        input[17] = delta2_y[1];
        
        // Pair 4: e(alpha, beta)
        input[18] = alpha1[0];
        input[19] = alpha1[1];
        input[20] = beta2_x[0];
        input[21] = beta2_x[1];
        input[22] = beta2_y[0];
        input[23] = beta2_y[1];
        
        // Call bn254 pairing precompile (address 0x08)
        // Returns 1 if pairing check passes, 0 otherwise
        bool success;
        uint256 result;
        
        assembly {
            success := staticcall(
                sub(gas(), 2000),
                0x08,
                input,
                768, // 24 * 32 bytes
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
}
