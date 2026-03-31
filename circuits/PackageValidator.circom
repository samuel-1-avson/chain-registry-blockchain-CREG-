pragma circom 2.1.0;

include "circomlib/poseidon.circom";
include "circomlib/comparators.circom";
include "circomlib/bitify.circom";

/// Circuit for validating package safety
/// 
/// Proves that a package meets safety criteria without revealing:
/// - The actual package content (only hash is public)
/// - Detailed analysis results (only pass/fail is public)
///
/// Public Inputs:
/// - contentHash: Poseidon hash of package content
/// - manifestHash: Hash of package manifest  
/// - staticAnalysisScore: Public safety score (0-100)
/// - sandboxPassed: Boolean (1 = passed)
/// - noVulnerableDeps: Boolean (1 = no vulns)
///
/// Private Inputs:
/// - content: Actual package content (for hash verification)
/// - complexityScore: Internal complexity metric
/// - networkCalls: Number of network calls detected
/// - fileWrites: Number of file writes detected

template PackageValidator(maxContentSize) {
    // Public inputs
    signal input contentHash;
    signal input manifestHash;
    signal input staticAnalysisScore;
    signal input sandboxPassed;
    signal input noVulnerableDeps;
    
    // Private inputs (witness)
    signal input content[maxContentSize];
    signal input complexityScore;
    signal input networkCalls;
    signal input fileWrites;
    
    // Output
    signal output isValid;
    
    // === Constraint 1: Verify content hash ===
    // Publisher proves they know content that hashes to contentHash
    component contentHasher = Poseidon(maxContentSize);
    for (var i = 0; i < maxContentSize; i++) {
        contentHasher.inputs[i] <== content[i];
    }
    contentHasher.out === contentHash;
    
    // === Constraint 2: Static analysis score >= 80 ===
    component scoreCheck = GreaterEqThan(8);
    scoreCheck.in[0] <== staticAnalysisScore;
    scoreCheck.in[1] <== 80;
    scoreCheck.out === 1;
    
    // === Constraint 3: Sandbox must pass ===
    sandboxPassed === 1;
    
    // === Constraint 4: No vulnerable dependencies ===
    noVulnerableDeps === 1;
    
    // === Constraint 5: Complexity score <= 90 ===
    component complexityCheck = LessEqThan(8);
    complexityCheck.in[0] <== complexityScore;
    complexityCheck.in[1] <== 90;
    complexityCheck.out === 1;
    
    // === Constraint 6: Limited network calls (<= 5) ===
    component networkCheck = LessEqThan(8);
    networkCheck.in[0] <== networkCalls;
    networkCheck.in[1] <== 5;
    networkCheck.out === 1;
    
    // === Constraint 7: Limited file writes (<= 10) ===
    component fileCheck = LessEqThan(8);
    fileCheck.in[0] <== fileWrites;
    fileCheck.in[1] <== 10;
    fileCheck.out === 1;
    
    // === Constraint 8: Overall safety calculation ===
    // SafetyScore = (staticAnalysisScore * 3 + complexityScore) / 4 >= 75
    // To avoid division, we check: staticAnalysisScore * 3 + complexityScore >= 300
    
    var weightedStatic = staticAnalysisScore * 3;
    signal totalScore <== weightedStatic + complexityScore;
    
    component overallCheck = GreaterEqThan(10);
    overallCheck.in[0] <== totalScore;
    overallCheck.in[1] <== 300;
    overallCheck.out === 1;
    
    // Output is valid only if all checks pass
    signal step1 <== scoreCheck.out * sandboxPassed;
    signal step2 <== step1 * noVulnerableDeps;
    signal step3 <== step2 * complexityCheck.out;
    signal step4 <== step3 * networkCheck.out;
    signal step5 <== step4 * fileCheck.out;
    isValid <== step5 * overallCheck.out;
    
    // Constraint: isValid must be 1 (all checks passed)
    isValid === 1;
}

/// Circuit for private package verification
/// 
/// Used for enterprise private registries where content must be kept confidential.
/// Only the hash and validation result are public.
template PrivatePackageValidator(maxContentSize) {
    // Public
    signal input contentHash;
    signal input validationResult;
    
    // Private
    signal input content[maxContentSize];
    signal input validatorSignature[64]; // ECDSA signature
    
    // Verify content hash
    component hasher = Poseidon(maxContentSize);
    for (var i = 0; i < maxContentSize; i++) {
        hasher.inputs[i] <== content[i];
    }
    hasher.out === contentHash;
    
    // Output
    signal output verified;
    verified <== validationResult;
}

/// Circuit for batch validation
/// 
/// Proves that multiple packages all pass validation efficiently.
template BatchPackageValidator(numPackages) {
    // Public: Array of content hashes
    signal input contentHashes[numPackages];
    signal input allValid;
    
    // Private: Array of validation results
    signal input validationResults[numPackages];
    
    // Verify each package is valid
    signal runningProduct[numPackages + 1];
    runningProduct[0] <== 1;
    
    for (var i = 0; i < numPackages; i++) {
        // Each package must be valid (1)
        validationResults[i] === 1;
        runningProduct[i + 1] <== runningProduct[i] * validationResults[i];
    }
    
    // Final product should be 1 only if all are valid
    allValid === runningProduct[numPackages];
    
    signal output batchVerified;
    batchVerified <== allValid;
}

/// Circuit for reputation-weighted validation
/// 
/// Validates that validators with sufficient reputation approved the package.
template ReputationWeightedValidator(numValidators) {
    // Public
    signal input packageHash;
    signal input totalReputation;
    signal input thresholdReputation;
    
    // Private
    signal input validatorReputations[numValidators];
    signal input approvals[numValidators]; // 1 = approved, 0 = not
    
    // Calculate weighted approvals
    signal weightedApprovals[numValidators];
    signal sum[numValidators + 1];
    sum[0] <== 0;
    
    for (var i = 0; i < numValidators; i++) {
        weightedApprovals[i] <== validatorReputations[i] * approvals[i];
        sum[i + 1] <== sum[i] + weightedApprovals[i];
    }
    
    // Verify threshold met
    component thresholdCheck = GreaterEqThan(64);
    thresholdCheck.in[0] <== sum[numValidators];
    thresholdCheck.in[1] <== thresholdReputation;
    thresholdCheck.out === 1;
    
    signal output approved;
    approved <== thresholdCheck.out;
}

// Main component instantiation
// Usage: circom PackageValidator.circom --r1cs --wasm --sym -p bn128 -l ./circomlib
component main {public [contentHash, manifestHash, staticAnalysisScore, sandboxPassed, noVulnerableDeps]} = PackageValidator(1024);
