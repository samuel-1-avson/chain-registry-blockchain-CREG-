// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "./Registry.sol";
import "./Staking.sol";
import "./Reputation.sol";

/// @title Appeal
/// @notice On-chain appeal mechanism for packages rejected by validator consensus.
/// @dev Publishers can stake an appeal bond to trigger a second review by a
///      human panel. If the appeal succeeds, the package is verified and the
///      bond is returned. If it fails, the bond is added to the slash pool.
///
/// Appeal lifecycle:
///   1. Publisher calls appeal() with bond payment
///   2. Governance-approved panelists cast votes (approve/reject)
///   3. Once the panel quorum is reached, the outcome is executed:
///      - Success: package verified on Registry, bond returned
///      - Failure: bond slashed, rejection stands
contract Appeal {

    // ── Structs ───────────────────────────────────────────────────────────────

    enum AppealStatus { Pending, Approved, Rejected, Expired }

    struct AppealRecord {
        string    canonical;
        address   publisher;
        uint256   bond;           // ETH bond posted with the appeal
        uint256   submittedAt;
        AppealStatus status;
        uint256   approveVotes;
        uint256   rejectVotes;
        string    publisherStatement; // Publisher's justification
        mapping(address => bool) voted;
    }

    // ── Storage ───────────────────────────────────────────────────────────────

    mapping(uint256 => AppealRecord) private _appeals;
    uint256 public appealCount;

    ChainRegistry public immutable registry;
    Staking       public immutable staking;
    Reputation    public immutable reputation;

    address public governance;

    /// Approved human panelists who can vote on appeals.
    mapping(address => bool) public isPanelist;
    /// Authorized AI Audit models that can resolve appeals instantly.
    mapping(address => bool) public isAIAuthorized;

    address[] public panelists;

    /// Minimum panel votes to decide an appeal.
    uint256 public panelQuorum = 3;

    /// Appeal bond (must be ≥ this to discourage frivolous appeals).
    uint256 public constant MIN_APPEAL_BOND = 0.1 ether;

    /// How long an appeal stays open before it expires.
    uint256 public constant APPEAL_WINDOW = 7 days;

    // ── Events ────────────────────────────────────────────────────────────────

    event AppealSubmitted(uint256 indexed id, string canonical, address publisher, uint256 bond);
    event AppealVoted    (uint256 indexed id, address panelist, bool approved);
    event AppealApproved (uint256 indexed id, string canonical);
    event AppealRejected (uint256 indexed id, string canonical);
    event AppealExpired  (uint256 indexed id, string canonical);
    event PanelistAdded  (address panelist);
    event PanelistRemoved(address panelist);

    // ── Errors ────────────────────────────────────────────────────────────────

    error BondTooLow(uint256 provided, uint256 minimum);
    error NotPanelist();
    error AlreadyVoted();
    error AppealNotPending();
    error AppealExpiredErr();
    error NotAuthorized();
    error NotGovernance();

    // ── Constructor ───────────────────────────────────────────────────────────

    constructor(address _registry, address _staking, address _reputation, address _governance) {
        registry   = ChainRegistry(_registry);
        staking    = Staking(_staking);
        reputation = Reputation(_reputation);
        governance = _governance;
    }


    // ── Publisher-facing ──────────────────────────────────────────────────────

    /// @notice Submit an appeal for a rejected package.
    /// @param canonical  The rejected package's canonical ID
    /// @param statement  Publisher's explanation / evidence
    function appeal(
        string calldata canonical,
        string calldata statement
    ) external payable returns (uint256 id) {
        if (msg.value < MIN_APPEAL_BOND)
            revert BondTooLow(msg.value, MIN_APPEAL_BOND);

        id = appealCount++;
        AppealRecord storage rec = _appeals[id];
        rec.canonical          = canonical;
        rec.publisher          = msg.sender;
        rec.bond               = msg.value;
        rec.submittedAt        = block.timestamp;
        rec.status             = AppealStatus.Pending;
        rec.publisherStatement = statement;

        emit AppealSubmitted(id, canonical, msg.sender, msg.value);
    }

    // ── Panel voting ──────────────────────────────────────────────────────────

    /// @notice A panelist votes on an appeal.
    function vote(uint256 id, bool approve) external {
        if (!isPanelist[msg.sender]) revert NotPanelist();

        AppealRecord storage rec = _appeals[id];
        if (rec.status != AppealStatus.Pending) revert AppealNotPending();
        if (block.timestamp > rec.submittedAt + APPEAL_WINDOW)
            revert AppealExpiredErr();
        if (rec.voted[msg.sender]) revert AlreadyVoted();

        rec.voted[msg.sender] = true;
        if (approve) { rec.approveVotes++; }
        else         { rec.rejectVotes++;  }

        emit AppealVoted(id, msg.sender, approve);

        // Auto-resolve once quorum is reached.
        if (rec.approveVotes >= panelQuorum) {
            _resolveApproved(id);
        } else if (rec.rejectVotes >= panelQuorum) {
            _resolveRejected(id);
        }
    }

    /// @notice Submit an automated verdict from an authorized AI Auditor.
    /// @dev Verifies the ECDSA signature of the AI provider before resolving.
    function submitAIVerdict(
        uint256 id,
        bool approve,
        bytes calldata signature
    ) external {
        AppealRecord storage rec = _appeals[id];
        if (rec.status != AppealStatus.Pending) revert AppealNotPending();

        // ── Verify AI Signature ───────────────────────────────────────────────
        bytes32 messageHash = keccak256(abi.encodePacked(id, approve, rec.canonical));
        bytes32 ethSignedMessageHash = keccak256(
            abi.encodePacked("\x19Ethereum Signed Message:\n32", messageHash)
        );

        address signer = recoverSigner(ethSignedMessageHash, signature);
        if (!isAIAuthorized[signer]) revert NotAuthorized();

        if (approve) {
            _resolveApproved(id);
        } else {
            _resolveRejected(id);
        }
    }

    // ── Cryptographic Helpers ─────────────────────────────────────────────────

    function recoverSigner(bytes32 _ethSignedMessageHash, bytes memory _signature)
        internal
        pure
        returns (address)
    {
        (bytes32 r, bytes32 s, uint8 v) = splitSignature(_signature);
        return ecrecover(_ethSignedMessageHash, v, r, s);
    }

    function splitSignature(bytes memory sig)
        internal
        pure
        returns (bytes32 r, bytes32 s, uint8 v)
    {
        require(sig.length == 65, "invalid signature length");
        assembly {
            r := mload(add(sig, 32))
            s := mload(add(sig, 64))
            v := byte(0, mload(add(sig, 96)))
        }
    }

    /// @notice Expire an appeal that has passed its window without a decision.
    function expireAppeal(uint256 id) external {
        AppealRecord storage rec = _appeals[id];
        if (rec.status != AppealStatus.Pending) revert AppealNotPending();
        if (block.timestamp <= rec.submittedAt + APPEAL_WINDOW)
            revert AppealNotPending();

        rec.status = AppealStatus.Expired;
        // Bond is forfeited on expiry.
        payable(governance).transfer(rec.bond);
        emit AppealExpired(id, rec.canonical);
    }

    // ── Internal resolution ───────────────────────────────────────────────────

    function _resolveApproved(uint256 id) internal {
        AppealRecord storage rec = _appeals[id];
        rec.status = AppealStatus.Approved;

        // Return bond to publisher.
        payable(rec.publisher).transfer(rec.bond);

        // Reward panelists who voted correctly (approve side).
        // In a full implementation, track per-panelist votes and reward
        // only those who voted with the majority.

        emit AppealApproved(id, rec.canonical);
    }

    function _resolveRejected(uint256 id) internal {
        AppealRecord storage rec = _appeals[id];
        rec.status = AppealStatus.Rejected;

        // Slash the bond — add to staking slash pool.
        staking.slash(rec.publisher, rec.bond, "Failed appeal");

        emit AppealRejected(id, rec.canonical);
    }

    // ── Governance ────────────────────────────────────────────────────────────

    function addPanelist(address panelist) external {
        if (msg.sender != governance) revert NotGovernance();
        require(!isPanelist[panelist], "Already a panelist");
        panelists.push(panelist);
        isPanelist[panelist] = true;
        emit PanelistAdded(panelist);
    }

    function removePanelist(address panelist) external {
        if (msg.sender != governance) revert NotGovernance();
        isPanelist[panelist] = false;
        emit PanelistRemoved(panelist);
    }

    function setPanelQuorum(uint256 q) external {
        if (msg.sender != governance) revert NotGovernance();
        require(q >= 1 && q <= panelists.length, "Invalid quorum");
        panelQuorum = q;
    }

    function authorizeAIModel(address model) external {
        if (msg.sender != governance) revert NotGovernance();
        isAIAuthorized[model] = true;
    }

    function revokeAIModel(address model) external {
        if (msg.sender != governance) revert NotGovernance();
        isAIAuthorized[model] = false;
    }

    // ── Views ─────────────────────────────────────────────────────────────────

    function getAppeal(uint256 id) external view returns (
        string memory canonical,
        address publisher,
        uint256 bond,
        AppealStatus status,
        uint256 approveVotes,
        uint256 rejectVotes,
        string memory statement
    ) {
        AppealRecord storage rec = _appeals[id];
        return (
            rec.canonical, rec.publisher, rec.bond,
            rec.status, rec.approveVotes, rec.rejectVotes,
            rec.publisherStatement
        );
    }

    function panelSize() external view returns (uint256) {
        return panelists.length;
    }
}
