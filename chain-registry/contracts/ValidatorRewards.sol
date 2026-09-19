// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "./Staking.sol";
import "./CregToken.sol";

/// @title ValidatorRewards
/// @notice Work-based reward system for validators - ONLY rewards actual verification work
/// @dev Validators are rewarded ONLY when they prove they verified a package (good or bad).
///      NO time-based rewards (no block rewards). NO daily limits. Pay for proven work only.
///      Base amounts are deliberately extremely small (0.00001 CREG range) so the 42M max supply
///      can sustain the network for a very long time even at massive verification volume.
///
/// REWARD MECHANISM (Mechanical Consensus):
/// 1. Validator votes on package (approve/reject) - NO reward yet
/// 2. Package reaches consensus and is finalized - Rewards distributed to participating validators
/// 3. Reward amount based on: 
///    - Number of packages verified (tiny base)
///    - Correctness of vote (higher reward for correct votes)
///    - Reputation score multiplier
///    - isMandatory flag (mandatory committee members get higher points base)
///
/// This creates a true "Proof of Verification" system where validators must do work to earn.
contract ValidatorRewards {
    
    // ── Constants ─────────────────────────────────────────────────────────────
    
    /// @notice Base reward per package verified (extremely low: 0.00001 CREG)
    /// @dev Paid ONLY for proven verification work that contributes to a finalized package.
    ///      Values kept tiny so the 42M max supply lasts for a very long time even with
    ///      tens or hundreds of millions of packages verified. This preserves high per-token value.
    uint256 public constant BASE_PACKAGE_REWARD = 0.00001 ether;
    
    /// @notice Bonus for validators who voted correctly (package verified = approved good, or rejected bad)
    uint256 public constant CORRECT_VOTE_BONUS = 0.000005 ether;
    
    /// @notice Penalty for validators who voted incorrectly (tiny deduction)
    /// @dev Still get base reward but less bonus
    uint256 public constant INCORRECT_VOTE_PENALTY = 0.000001 ether;
    
    /// @notice Minimum reward per verified package (can't go below this)
    uint256 public constant MIN_PACKAGE_REWARD = 0.000001 ether;
    
    // ── Storage ───────────────────────────────────────────────────────────────
    
    Staking public immutable staking;
    CregToken public immutable cregToken;
    address public governance;
    address public treasury;
    
    /// @notice Tracks validator rewards (validator => pending reward)
    mapping(address => uint256) public pendingRewards;
    
    /// @notice Tracks packages verified by each validator (validator => count)
    mapping(address => uint256) public packagesVerified;
    
    /// @notice Tracks correct votes by validator (validator => correct count)
    mapping(address => uint256) public correctVotes;
    
    /// @notice Tracks incorrect votes by validator (validator => incorrect count)
    mapping(address => uint256) public incorrectVotes;
    
    /// @notice Total rewards distributed
    uint256 public totalRewardsDistributed;
    
    /// @notice Total packages verified through this system
    uint256 public totalPackagesVerified;
    
    // ── Events ─────────────────────────────────────────────────────────────────
    
    event WorkReward(
        address indexed validator, 
        uint256 amount, 
        string packageCanonical,
        bool votedCorrectly,
        uint256 reputationBonus
    );
    event RewardsClaimed(address indexed validator, uint256 amount);
    event TreasuryFunded(address indexed source, uint256 amount);

    // === HYBRID ANALYSIS COMMITTEE EVENTS ===
    /// @notice Emitted when a validator contributes analysis (mandatory or voluntary) to a package.
    event AnalysisContribution(
        bytes32 indexed canonical,      // keccak256 of the package canonical
        address indexed validator,
        bool isMandatory,
        uint8 qualityScore,             // 0-100 derived from model/findings/agreement
        uint256 pointsAwarded,
        uint256 timestamp,
        bytes32 evidenceDigest
    );

    /// @notice Emitted when a package's hybrid analysis committee is finalized on L1.
    event AnalysisCommitteeFinalized(
        bytes32 indexed canonical,
        uint32 mandatorySize,
        uint32 effectiveSize,
        uint32 mandatoryContributed,
        uint32 voluntaryContributed,
        bytes32 selectionSeed
    );
    
    // ── Errors ─────────────────────────────────────────────────────────────────
    
    error NotAuthorized();
    error NotActiveValidator();
    error NoRewardsToClaim();
    error InvalidVoteDistribution();
    
    // ── Constructor ────────────────────────────────────────────────────────────
    
    constructor(
        address _staking,
        address _cregToken,
        address _governance,
        address _treasury
    ) {
        staking = Staking(_staking);
        cregToken = CregToken(_cregToken);
        governance = _governance;
        treasury = _treasury;
    }
    
    // ── Work-Based Reward Distribution ─────────────────────────────────────────
    
    /// @notice Distribute rewards to validators who verified a package.
    /// @dev Called by Registry (via bridge) when package reaches consensus. 
    ///      Rewards are based on PROOF OF WORK (actual verification) using extremely low per-package amounts
    ///      so the fixed 42M supply lasts across a very long horizon.
    /// @param packageCanonical The canonical ID of the verified package
    /// @param validators Array of validator addresses who participated in verification
    /// @param approvals Array of bool indicating if each validator approved (true) or rejected (false)
    /// @param packageWasGood Boolean indicating if the package was actually good (true) or bad/false positive (false)
    /// @param reputationScores Array of reputation scores (0-100) for each validator
    /// @param isMandatory Array indicating if each validator was in the mandatory committee (hybrid model)
    struct ValidatorWorkItem {
        address validator;
        bool approved;
        uint256 reputationScore;
        bool isMandatory;
    }

    function distributeWorkRewards(
        string calldata packageCanonical,
        address[] calldata validators,
        bool[] calldata approvals,
        bool packageWasGood,
        uint256[] calldata reputationScores,
        bool[] calldata isMandatory
    ) external {
        if (msg.sender != governance && msg.sender != address(staking)) revert NotAuthorized();
        if (validators.length != approvals.length || validators.length != reputationScores.length || validators.length != isMandatory.length) 
            revert InvalidVoteDistribution();
        if (validators.length == 0) revert InvalidVoteDistribution();
        
        totalPackagesVerified++;
        
        for (uint i = 0; i < validators.length; i++) {
            ValidatorWorkItem memory item = ValidatorWorkItem({
                validator: validators[i],
                approved: approvals[i],
                reputationScore: reputationScores[i],
                isMandatory: isMandatory[i]
            });
            _processOne(item, packageWasGood, packageCanonical);
        }
        
        // Emit committee finalization (simplified; real call would pass sizes)
        emit AnalysisCommitteeFinalized(
            keccak256(bytes(packageCanonical)),
            0, // filled by caller in practice
            0,
            0,
            0,
            bytes32(0)
        );
    }

    function _processOne(
        ValidatorWorkItem memory item,
        bool packageWasGood,
        string calldata packageCanonical
    ) internal {
        _processValidatorReward(
            item.validator,
            item.approved,
            packageWasGood,
            item.reputationScore,
            packageCanonical
        );

        uint8 quality = _computeQualityScore(item.reputationScore, item.approved, packageWasGood);
        uint256 points = _calculatePoints(item.approved, item.reputationScore, item.isMandatory);
        emit AnalysisContribution(
            keccak256(bytes(packageCanonical)),
            item.validator,
            item.isMandatory,
            quality,
            points,
            block.timestamp,
            bytes32(0)
        );
    }

    function _computeQualityScore(uint256 rep, bool approved, bool wasGood) internal pure returns (uint8) {
        uint8 base = uint8(rep > 100 ? 100 : rep);
        if ((wasGood && approved) || (!wasGood && !approved)) {
            return base > 80 ? 100 : base + 20;
        }
        return base / 2;
    }

    /// @notice Calculate points awarded for an AnalysisContribution (used for reputation/selection feedback + events).
    /// Base is deliberately low (25 mandatory / 10 voluntary) so that points remain meaningful
    /// even after millions of verifications. Mandatory committee members receive the higher base.
    function _calculatePoints(bool votedCorrectly, uint256 rep, bool isMand) internal pure returns (uint256) {
        uint256 base = isMand ? 25 : 10;
        if (votedCorrectly) base = base * 12 / 10;
        return base * (rep > 100 ? 100 : rep) / 100;
    }
    
    /// @dev Process reward for a single validator (extracted to avoid stack-too-deep)
    function _processValidatorReward(
        address validator,
        bool validatorApproved,
        bool packageWasGood,
        uint256 repScore,
        string calldata packageCanonical
    ) internal {
        if (!staking.isActiveValidator(validator)) return;
        
        bool votedCorrectly = (packageWasGood && validatorApproved) || (!packageWasGood && !validatorApproved);
        uint256 reward = _calculateWorkReward(votedCorrectly, repScore);
        
        pendingRewards[validator] += reward;
        packagesVerified[validator]++;
        
        if (votedCorrectly) {
            correctVotes[validator]++;
        } else {
            incorrectVotes[validator]++;
        }
        
        totalRewardsDistributed += reward;
        require(cregToken.transferFrom(treasury, address(this), reward), "Treasury transfer failed");
        
        emit WorkReward(validator, reward, packageCanonical, votedCorrectly, repScore);
    }
    
    /// @notice Calculate reward for a single verification.
    /// @param votedCorrectly Whether the validator voted correctly
    /// @param reputationScore Validator's reputation score (0-100)
    /// @return reward Amount of CREG earned
    function _calculateWorkReward(bool votedCorrectly, uint256 reputationScore) 
        internal 
        pure 
        returns (uint256 reward) 
    {
        // Base reward for doing the work (verifying)
        reward = BASE_PACKAGE_REWARD;
        
        if (votedCorrectly) {
            // Bonus for correct vote
            reward += CORRECT_VOTE_BONUS;
            
            // Additional bonus based on reputation (0-20% extra)
            uint256 repBonus = (BASE_PACKAGE_REWARD * reputationScore) / 500; // max 20%
            reward += repBonus;
        } else {
            // Penalty for incorrect vote
            if (reward > INCORRECT_VOTE_PENALTY) {
                reward -= INCORRECT_VOTE_PENALTY;
            } else {
                reward = MIN_PACKAGE_REWARD; // Floor
            }
        }
        
        return reward;
    }
    
    /// @notice Simplified version: equal base reward, no reputation bonus
    /// @dev Used when reputation system is not available or for testing
    function distributeSimpleWorkRewards(
        string calldata packageCanonical,
        address[] calldata validators,
        bool[] calldata approvals,
        bool packageWasGood
    ) external {
        if (msg.sender != governance && msg.sender != address(staking)) revert NotAuthorized();
        if (validators.length != approvals.length) revert InvalidVoteDistribution();
        if (validators.length == 0) revert InvalidVoteDistribution();
        
        totalPackagesVerified++;
        uint256 rewardPerValidator = BASE_PACKAGE_REWARD / validators.length;
        if (rewardPerValidator < MIN_PACKAGE_REWARD) rewardPerValidator = MIN_PACKAGE_REWARD;
        
        uint256 totalReward = rewardPerValidator * validators.length;
        require(cregToken.transferFrom(treasury, address(this), totalReward), "Treasury transfer failed");
        
        for (uint i = 0; i < validators.length; i++) {
            _processSimpleReward(
                validators[i],
                approvals[i],
                packageWasGood,
                rewardPerValidator,
                packageCanonical
            );
        }
        
        totalRewardsDistributed += totalReward;
    }
    
    /// @dev Process simple reward for a single validator (extracted to avoid stack-too-deep)
    function _processSimpleReward(
        address validator,
        bool validatorApproved,
        bool packageWasGood,
        uint256 rewardPerValidator,
        string calldata packageCanonical
    ) internal {
        if (!staking.isActiveValidator(validator)) return;
        
        bool votedCorrectly = (packageWasGood && validatorApproved) || (!packageWasGood && !validatorApproved);
        
        uint256 reward = votedCorrectly ? rewardPerValidator + CORRECT_VOTE_BONUS : rewardPerValidator - INCORRECT_VOTE_PENALTY;
        if (reward < MIN_PACKAGE_REWARD) reward = MIN_PACKAGE_REWARD;
        
        pendingRewards[validator] += reward;
        packagesVerified[validator]++;
        
        if (votedCorrectly) {
            correctVotes[validator]++;
        } else {
            incorrectVotes[validator]++;
        }
        
        emit WorkReward(validator, reward, packageCanonical, votedCorrectly, 50);
    }
    
    // ── Claim Rewards ──────────────────────────────────────────────────────────
    
    /// @notice Validators claim their accumulated rewards from verified work
    function claimRewards() external {
        uint256 amount = pendingRewards[msg.sender];
        if (amount == 0) revert NoRewardsToClaim();
        
        pendingRewards[msg.sender] = 0;
        
        require(cregToken.transfer(msg.sender, amount), "Reward transfer failed");
        
        emit RewardsClaimed(msg.sender, amount);
    }
    
    /// @notice View pending rewards for a validator
    function getPendingRewards(address validator) external view returns (uint256) {
        return pendingRewards[validator];
    }
    
    // ── View Functions ────────────────────────────────────────────────────────
    
    /// @notice Get validator statistics
    function getValidatorStats(address validator) external view returns (
        uint256 _packagesVerified,
        uint256 _correctVotes,
        uint256 _incorrectVotes,
        uint256 accuracyRate
    ) {
        _packagesVerified = packagesVerified[validator];
        _correctVotes = correctVotes[validator];
        _incorrectVotes = incorrectVotes[validator];
        
        uint256 total = _correctVotes + _incorrectVotes;
        accuracyRate = total > 0 ? (_correctVotes * 100) / total : 0;
    }
    
    /// @notice Estimate earnings based on verification volume (amounts are tiny by design).
    /// With the current low per-package rewards, even high daily volume yields very small CREG amounts.
    /// This scarcity is intentional to keep long-term token value high.
    /// @param packagesPerDay Number of packages validator expects to verify daily
    /// @param accuracyPercent Expected accuracy rate (0-100)
    /// @param reputationScore Validator's reputation score (0-100)
    function estimateEarnings(
        uint256 packagesPerDay,
        uint256 accuracyPercent,
        uint256 reputationScore
    ) external pure returns (uint256 daily, uint256 monthly, uint256 yearly) {
        // Calculate average reward per package based on accuracy (base values are extremely low)
        uint256 avgReward = BASE_PACKAGE_REWARD;
        
        if (accuracyPercent >= 80) {
            // High accuracy - get bonuses
            avgReward += CORRECT_VOTE_BONUS;
            avgReward += (BASE_PACKAGE_REWARD * reputationScore) / 500; // rep bonus (still tiny)
        } else if (accuracyPercent >= 50) {
            // Medium accuracy - partial bonus
            avgReward += CORRECT_VOTE_BONUS / 2;
        } else {
            // Low accuracy - penalty
            avgReward = avgReward > INCORRECT_VOTE_PENALTY ? avgReward - INCORRECT_VOTE_PENALTY : MIN_PACKAGE_REWARD;
        }
        
        daily = packagesPerDay * avgReward;
        monthly = daily * 30;
        yearly = daily * 365;
    }
    
    // ── Admin Functions ───────────────────────────────────────────────────────
    
    /// @notice Fund the treasury (called by governance or token contract)
    function fundTreasury(uint256 amount) external {
        require(cregToken.transferFrom(msg.sender, treasury, amount), "Funding failed");
        emit TreasuryFunded(msg.sender, amount);
    }
    
    /// @notice Update treasury address
    function setTreasury(address newTreasury) external {
        if (msg.sender != governance) revert NotAuthorized();
        treasury = newTreasury;
    }
    
    /// @notice Emergency withdrawal of stuck tokens (governance only)
    function emergencyWithdraw(address token, uint256 amount) external {
        if (msg.sender != governance) revert NotAuthorized();
        require(CregToken(token).transfer(governance, amount), "Emergency withdraw failed");
    }
}
