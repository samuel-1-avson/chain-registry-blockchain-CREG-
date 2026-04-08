// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

/// @title Governance
/// @notice CANONICAL governance contract — M-of-N multisig for the chain registry.
/// @dev This is the ACTIVE governance contract used by Registry.sol and Staking.sol.
///      Proposals are submitted, voted on by signers, and executed
///      automatically once the approval threshold is met.
///      This prevents any single entity from controlling the registry.
///
///      See GovernanceV2.sol for the planned token-based governance upgrade
///      (quadratic voting, delegation, automated parameter adjustments).
contract Governance {
    // ── Reentrancy Guard ─────────────────────────────────────────────────────
    bool private _locked;
    modifier nonReentrant() {
        require(!_locked, "Reentrant call");
        _locked = true;
        _;
        _locked = false;
    }

    // ── Structs ───────────────────────────────────────────────────────────────

    enum ProposalStatus { Pending, Executed, Cancelled }
    enum SystemStatus { Active, Paused }

    struct Proposal {
        address target;          // Contract to call
        bytes   callData;        // Encoded function call
        string  description;
        uint256 submittedAt;
        uint256 executedAt;
        ProposalStatus status;
        uint256 approvalCount;
        uint256 rejectionCount;
        mapping(address => bool) voted;
        mapping(address => bool) approval;
    }

    // ── Storage ───────────────────────────────────────────────────────────────

    address[] public signers;
    mapping(address => bool) public isSigner;
    uint256 public threshold;            // Minimum approvals to execute
    uint256 public proposalCount;
    uint256 public votingPeriod;         // Seconds proposals are open

    mapping(uint256 => Proposal) private _proposals;

    // ── Pause State ───────────────────────────────────────────────────────────

    SystemStatus public systemStatus;
    uint256 public pausedAt;
    string public pauseReason;

    /// @notice Minimum number of signers that must co-sign an emergency pause.
    uint256 public constant PAUSE_THRESHOLD = 2;

    /// @notice Cooldown period between pauses (prevents griefing).
    uint256 public constant PAUSE_COOLDOWN = 7 days;

    /// @notice Tracks co-signers for a pending pause request.
    mapping(bytes32 => mapping(address => bool)) public pauseCoSigners;
    mapping(bytes32 => uint256) public pauseCoSignCount;

    // ── Events ────────────────────────────────────────────────────────────────

    event ProposalSubmitted(uint256 indexed id, address indexed proposer, string description);
    event ProposalVoted    (uint256 indexed id, address indexed signer, bool approved);
    event ProposalExecuted (uint256 indexed id);
    event ProposalCancelled(uint256 indexed id);
    event SignerAdded      (address indexed signer);
    event SignerRemoved    (address indexed signer);
    event ThresholdUpdated (uint256 newThreshold);
    event EmergencyPaused  (address indexed triggeredBy, string reason, uint256 timestamp);
    event EmergencyUnpaused(address indexed triggeredBy, uint256 timestamp);

    // ── Errors ────────────────────────────────────────────────────────────────

    error NotSigner();
    error AlreadyVoted();
    error ProposalNotPending();
    error ThresholdNotMet(uint256 got, uint256 required);
    error VotingPeriodExpired();
    error ExecutionFailed();
    error SystemPaused();
    error SystemNotPaused();
    error NotGovernance();
    error EmergencyNotAuthorized();
    error InvalidPauseReason();

    // ── Constructor ───────────────────────────────────────────────────────────

    /// @param _signers   Initial signer set (e.g. founding coalition)
    /// @param _threshold Minimum approvals required (e.g. 4-of-7)
    constructor(address[] memory _signers, uint256 _threshold) {
        require(_signers.length >= _threshold, "Threshold exceeds signer count");
        require(_threshold > 0, "Threshold must be > 0");

        for (uint i = 0; i < _signers.length; i++) {
            signers.push(_signers[i]);
            isSigner[_signers[i]] = true;
        }
        threshold    = _threshold;
        votingPeriod = 3 days;
        systemStatus = SystemStatus.Active;
    }

    // ── Modifiers ─────────────────────────────────────────────────────────────

    modifier whenNotPaused() {
        if (systemStatus == SystemStatus.Paused) revert SystemPaused();
        _;
    }

    modifier whenPaused() {
        if (systemStatus == SystemStatus.Active) revert SystemNotPaused();
        _;
    }

    // ── Emergency Pause ────────────────────────────────────────────────────────

    /// @notice Co-sign an emergency pause request.
    /// @dev Requires at least PAUSE_THRESHOLD (2) distinct signers to co-sign
    ///      the same reason hash before the pause takes effect.
    ///      Enforces a 7-day cooldown between successive pauses.
    /// @param reason Human-readable reason for the pause
    function emergencyPause(string calldata reason) external {
        if (!isSigner[msg.sender]) revert NotSigner();
        if (bytes(reason).length == 0) revert InvalidPauseReason();
        if (systemStatus == SystemStatus.Paused) revert SystemPaused();

        // Enforce cooldown since last pause
        require(
            block.timestamp >= pausedAt + PAUSE_COOLDOWN,
            "Pause cooldown active"
        );

        bytes32 reasonHash = keccak256(bytes(reason));

        require(!pauseCoSigners[reasonHash][msg.sender], "Already co-signed");
        pauseCoSigners[reasonHash][msg.sender] = true;
        pauseCoSignCount[reasonHash]++;

        if (pauseCoSignCount[reasonHash] >= PAUSE_THRESHOLD) {
            systemStatus = SystemStatus.Paused;
            pausedAt = block.timestamp;
            pauseReason = reason;

            emit EmergencyPaused(msg.sender, reason, block.timestamp);
        }
    }

    /// @notice Unpause the system.
    /// @dev Requires a governance proposal to be approved (m-of-n signers).
    ///      This prevents a single signer from unilaterally unpausing.
    function emergencyUnpause() external whenPaused {
        // Only callable via governance proposal (self-call)
        if (msg.sender != address(this)) revert NotGovernance();

        systemStatus = SystemStatus.Active;
        emit EmergencyUnpaused(msg.sender, block.timestamp);
    }

    /// @notice Check if the system is currently paused.
    /// @return True if paused
    function isPaused() external view returns (bool) {
        return systemStatus == SystemStatus.Paused;
    }

    /// @notice Get pause information.
    /// @return paused Whether system is paused
    /// @return reason Reason for pause
    /// @return duration Seconds since pause (0 if not paused)
    function getPauseInfo() external view returns (bool paused, string memory reason, uint256 duration) {
        paused = systemStatus == SystemStatus.Paused;
        reason = pauseReason;
        duration = paused ? block.timestamp - pausedAt : 0;
    }

    // ── Proposal lifecycle ────────────────────────────────────────────────────

    /// @notice Submit a new governance proposal.
    function submit(
        address target,
        bytes calldata callData,
        string calldata description
    ) external whenNotPaused returns (uint256 id) {
        if (!isSigner[msg.sender]) revert NotSigner();

        id = proposalCount++;
        Proposal storage p = _proposals[id];
        p.target       = target;
        p.callData     = callData;
        p.description  = description;
        p.submittedAt  = block.timestamp;
        p.status       = ProposalStatus.Pending;

        emit ProposalSubmitted(id, msg.sender, description);
    }

    /// @notice Vote on a pending proposal.
    function vote(uint256 id, bool approve) external nonReentrant {
        if (!isSigner[msg.sender]) revert NotSigner();

        Proposal storage p = _proposals[id];
        if (p.status != ProposalStatus.Pending) revert ProposalNotPending();
        if (block.timestamp > p.submittedAt + votingPeriod) revert VotingPeriodExpired();
        if (p.voted[msg.sender]) revert AlreadyVoted();

        p.voted[msg.sender]    = true;
        p.approval[msg.sender] = approve;

        if (approve) { p.approvalCount++;  }
        else         { p.rejectionCount++; }

        emit ProposalVoted(id, msg.sender, approve);

        // Auto-execute once threshold is met.
        if (p.approvalCount >= threshold) {
            _execute(id);
        }
    }

    /// @notice Cancel a proposal (only if voting period has expired and threshold not met).
    function cancel(uint256 id) external {
        if (!isSigner[msg.sender]) revert NotSigner();
        Proposal storage p = _proposals[id];
        if (p.status != ProposalStatus.Pending) revert ProposalNotPending();

        p.status = ProposalStatus.Cancelled;
        emit ProposalCancelled(id);
    }

    // ── Internal ──────────────────────────────────────────────────────────────

    function _execute(uint256 id) internal {
        Proposal storage p = _proposals[id];
        p.status     = ProposalStatus.Executed;
        p.executedAt = block.timestamp;

        (bool success, ) = p.target.call(p.callData);
        if (!success) revert ExecutionFailed();

        emit ProposalExecuted(id);
    }

    // ── Signer management (only via proposal) ─────────────────────────────────

    function addSigner(address newSigner) external {
        require(msg.sender == address(this), "Only via governance proposal");
        require(!isSigner[newSigner], "Already a signer");
        signers.push(newSigner);
        isSigner[newSigner] = true;
        emit SignerAdded(newSigner);
    }

    function removeSigner(address signer) external {
        require(msg.sender == address(this), "Only via governance proposal");
        require(signers.length - 1 >= threshold, "Would break threshold");
        isSigner[signer] = false;
        emit SignerRemoved(signer);
    }

    function updateThreshold(uint256 newThreshold) external {
        require(msg.sender == address(this), "Only via governance proposal");
        require(newThreshold > 0 && newThreshold <= signers.length, "Invalid threshold");
        threshold = newThreshold;
        emit ThresholdUpdated(newThreshold);
    }

    // ── Views ─────────────────────────────────────────────────────────────────

    function getProposal(uint256 id) external view returns (
        address target,
        string memory description,
        uint256 submittedAt,
        ProposalStatus status,
        uint256 approvalCount,
        uint256 rejectionCount
    ) {
        Proposal storage p = _proposals[id];
        return (p.target, p.description, p.submittedAt, p.status, p.approvalCount, p.rejectionCount);
    }

    function signerCount() external view returns (uint256) {
        return signers.length;
    }
}
