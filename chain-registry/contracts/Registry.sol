// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "./Staking.sol";
import "./Reputation.sol";
import "./VRF.sol";
import "./Governance.sol";
import "./ZKVerifier.sol";

/// @title ChainRegistry
/// @notice Core on-chain package registry with ZK proof support
/// @dev Records publish/revoke events. Supports both PBFT consensus
///      and ZK proof validation for faster verification.
contract ChainRegistry {

    // ── Reentrancy Guard ─────────────────────────────────────────────────────
    bool private _locked;
    modifier nonReentrant() {
        require(!_locked, "Reentrant call");
        _locked = true;
        _;
        _locked = false;
    }

    // ── Structs ───────────────────────────────────────────────────────────────

    enum PackageStatus { Unknown, Pending, Verified, Revoked }
    enum ValidationMode { PBFT, ZKProof }

    struct PackageRecord {
        string  canonical;         // "ecosystem:name@version"
        bytes32 contentHash;       // SHA-256 of the tarball
        string  ipfsCid;           // IPFS CID of the tarball
        address publisher;
        uint64  publishedAt;       // Unix timestamp
        bytes32 blockHash;         // Block hash that included this record
        PackageStatus status;
        string  revocationReason;
        ValidationMode validationMode;
        bytes32 zkProofHash;       // Hash of ZK proof (if ZK validated)
    }

    struct ValidatorSig {
        address validator;
        bytes   signature;         // ECDSA sig over (canonical ++ contentHash)
        bool    approved;
    }

    struct ZKProofData {
        uint256[8] proof;          // Groth16 proof points
        uint256[] publicInputs;    // Public inputs to verify
        uint64    verifiedAt;      // Timestamp of verification
    }

    // ── Storage ───────────────────────────────────────────────────────────────

    /// canonical → PackageRecord
    mapping(bytes32 => PackageRecord) public packages;

    /// canonical → list of validator signatures from the consensus round
    mapping(bytes32 => ValidatorSig[]) public consensusProofs;

    /// canonical → ZK proof data
    mapping(bytes32 => ZKProofData) public zkProofs;

    Staking    public immutable staking;
    Reputation public immutable reputation;
    VRF        public immutable vrf;
    ZKVerifier public immutable zkVerifier;

    address public governance;   // Multisig / DAO that can update rules.
    uint8   public quorumPct;    // Minimum approval percentage (default 67).
    
    // Validation configuration
    bool public zkValidationEnabled;
    uint256 public zkValidationFee;  // Fee for ZK validation (cheaper than PBFT)

    // ── L2 Rollup State ────────────────────────────────────────────────────────
    
    bytes32 public latestStateRoot;  // Merkle root of the entire verified registry
    uint256 public totalBatches;      // Counter for rollup batches submitted to L1
    
    /// batchId → stateRoot
    mapping(uint256 => bytes32) public batchRoots;
    
    /// batchId → transactionDataRoot (for Data Availability)
    mapping(uint256 => bytes32) public batchDataRoots;

    // ── Events ────────────────────────────────────────────────────────────────

    event PackageSubmitted(bytes32 indexed key, string canonical, address indexed publisher);
    event PackageVerified (bytes32 indexed key, string canonical, uint validatorCount);
    event PackageVerifiedZK(bytes32 indexed key, string canonical, bytes32 proofHash);
    event PackageRevoked  (bytes32 indexed key, string canonical, string reason);
    event GovernanceUpdated(address newGovernance);
    event QuorumUpdated(uint8 newQuorumPct);
    event ZKValidationToggled(bool enabled);
    
    event BatchSubmitted(
        uint256 indexed batchId,
        bytes32 prevStateRoot,
        bytes32 nextStateRoot,
        uint256 txCount,
        bytes32 dataRoot
    );

    // ── Errors ────────────────────────────────────────────────────────────────

    error AlreadyExists(string canonical);
    error NotFound(string canonical);
    error AlreadyRevoked(string canonical);
    error InsufficientQuorum(uint got, uint required);
    error InvalidSignature(address validator);
    error InvalidZKProof();
    error NotGovernance();
    error NotPublisher();
    error ZKDisabled();

    // ── Modifiers ─────────────────────────────────────────────────────────────

    modifier onlyGovernance() {
        if (msg.sender != governance) revert NotGovernance();
        _;
    }

    modifier whenNotPaused() {
        if (Governance(governance).isPaused()) {
            revert("System is paused");
        }
        _;
    }

    // ── Constructor ───────────────────────────────────────────────────────────

    constructor(
        address _staking,
        address _reputation,
        address _vrf,
        address _governance,
        address _zkVerifier
    ) {
        staking    = Staking(_staking);
        reputation = Reputation(_reputation);
        vrf        = VRF(_vrf);
        zkVerifier = ZKVerifier(_zkVerifier);
        governance = _governance;
        quorumPct  = 67; // 2/3 majority
        zkValidationEnabled = true;
        zkValidationFee = 0.001 ether; // Cheaper than PBFT
    }

    // ── Publisher-facing ──────────────────────────────────────────────────────

    /// @notice Submit a package to the pending pool.
    /// @param canonical  e.g. "npm:express@4.18.2"
    /// @param contentHash SHA-256 of the tarball bytes
    /// @param ipfsCid    IPFS CID where the tarball is pinned
    function submitPackage(
        string calldata canonical,
        bytes32 contentHash,
        string calldata ipfsCid
    ) external whenNotPaused {
        bytes32 key = _key(canonical);
        PackageRecord storage rec = packages[key];

        if (rec.status != PackageStatus.Unknown) {
            revert AlreadyExists(canonical);
        }

        // Publisher must have staked tokens to publish.
        require(staking.stakedBalance(msg.sender) > 0, "Publisher must stake first");

        packages[key] = PackageRecord({
            canonical:         canonical,
            contentHash:       contentHash,
            ipfsCid:           ipfsCid,
            publisher:         msg.sender,
            publishedAt:       uint64(block.timestamp),
            // Note: blockhash() only works for the 256 most recent blocks.
            // Older packages will have blockHash = 0x0. This is acceptable
            // as it serves as an inclusion record, not a randomness source.
            blockHash:         blockhash(block.number - 1),
            status:            PackageStatus.Pending,
            revocationReason:  "",
            validationMode:    ValidationMode.PBFT,
            zkProofHash:       bytes32(0)
        });

        emit PackageSubmitted(key, canonical, msg.sender);
    }
    
    /// @notice Submit a package with ZK proof for instant verification
    /// @param canonical  Package canonical ID
    /// @param contentHash SHA-256 of the tarball
    /// @param ipfsCid    IPFS CID
    /// @param proof      Groth16 proof (8 uint256 values)
    /// @param publicInputs Public inputs for verification
    function submitPackageWithZKProof(
        string calldata canonical,
        bytes32 contentHash,
        string calldata ipfsCid,
        uint256[8] calldata proof,
        uint256[] calldata publicInputs
    ) external payable whenNotPaused {
        if (!zkValidationEnabled) revert ZKDisabled();
        if (msg.value < zkValidationFee) revert("Insufficient fee");
        
        bytes32 key = _key(canonical);
        PackageRecord storage rec = packages[key];

        if (rec.status != PackageStatus.Unknown) {
            revert AlreadyExists(canonical);
        }

        // Publisher must have staked tokens
        require(staking.stakedBalance(msg.sender) > 0, "Publisher must stake first");
        
        // Verify ZK proof on-chain
        bool proofValid = zkVerifier.verifyProof(proof, publicInputs);
        if (!proofValid) revert InvalidZKProof();
        
        // Compute proof hash for record
        bytes32 proofHash = keccak256(abi.encodePacked(proof, publicInputs));

        packages[key] = PackageRecord({
            canonical:         canonical,
            contentHash:       contentHash,
            ipfsCid:           ipfsCid,
            publisher:         msg.sender,
            publishedAt:       uint64(block.timestamp),
            blockHash:         blockhash(block.number - 1), // See note in submitPackage()
            status:            PackageStatus.Verified,
            revocationReason:  "",
            validationMode:    ValidationMode.ZKProof,
            zkProofHash:       proofHash
        });
        
        // Store ZK proof data
        zkProofs[key] = ZKProofData({
            proof: proof,
            publicInputs: publicInputs,
            verifiedAt: uint64(block.timestamp)
        });

        emit PackageVerifiedZK(key, canonical, proofHash);
        
        // Refund excess fee
        if (msg.value > zkValidationFee) {
            payable(msg.sender).transfer(msg.value - zkValidationFee);
        }
    }

    // ── L2 Rollup Settlement ─────────────────────────────────────────────────

    /// @notice Submit a batch of package verifications to achieve L2 finality.
    /// @param prevRoot Previous state root (must match latestStateRoot)
    /// @param nextRoot New state root after processing the batch
    /// @param txCount  Number of verified packages in this batch
    /// @param dataRoot Merkle root of the transaction data (for DA)
    /// @param proof    Validity proof (ZK-SNARK) confirming the state transition
    function submitRollupBatch(
        bytes32 prevRoot,
        bytes32 nextRoot,
        uint256 txCount,
        bytes32 dataRoot,
        uint256[8] calldata proof,
        uint256[] calldata publicInputs
    ) external onlyGovernance {
        // 1. Verify previous state root matches
        require(prevRoot == latestStateRoot, "Invalid previous state root");
        
        // 2. Verify validity proof (Validity Rollup)
        // In a true L2, the ZK proof confirms that the transition from prev -> next
        // is valid according to the protocol rules.
        bool proofValid = zkVerifier.verifyProof(proof, publicInputs);
        if (!proofValid) revert InvalidZKProof();
        
        // 3. Update state
        totalBatches++;
        latestStateRoot = nextRoot;
        batchRoots[totalBatches] = nextRoot;
        batchDataRoots[totalBatches] = dataRoot;
        
        emit BatchSubmitted(totalBatches, prevRoot, nextRoot, txCount, dataRoot);
    }

    // resetRollupState() has been REMOVED for security.
    // It previously allowed governance to wipe all L2 history on L1.
    // This is unacceptable in production — L2 state must be immutable once committed.

    // ── Consensus-facing ─────────────────────────────────────────────────────

    /// @notice Finalize a package after PBFT consensus.
    /// @dev Called by the chain node once the off-chain PBFT round completes.
    function finalizePackage(
        string calldata canonical,
        ValidatorSig[] calldata sigs
    ) external whenNotPaused {
        bytes32 key = _key(canonical);
        PackageRecord storage rec = packages[key];

        if (rec.status == PackageStatus.Unknown) revert NotFound(canonical);
        if (rec.status == PackageStatus.Revoked)  revert AlreadyRevoked(canonical);

        uint activeValidators = staking.activeValidatorCount();
        uint required = (activeValidators * quorumPct) / 100 + 1;

        // Verify each signature and count approvals.
        uint approvals;
        bytes32 digest = _sigDigest(canonical, rec.contentHash);

        for (uint i = 0; i < sigs.length; i++) {
            ValidatorSig calldata s = sigs[i];

            // Validator must be staked and active.
            if (!staking.isActiveValidator(s.validator)) continue;

            // Verify ECDSA signature over (canonical ++ contentHash).
            address recovered = _recoverSigner(digest, s.signature);
            if (recovered != s.validator) revert InvalidSignature(s.validator);

            consensusProofs[key].push(s);
            if (s.approved) {
                approvals++;
                reputation.recordApproval(s.validator);
            } else {
                reputation.recordRejection(s.validator);
            }
        }

        if (approvals < required) {
            // Consensus failed — package stays Pending (can be appealed).
            revert InsufficientQuorum(approvals, required);
        }

        rec.status    = PackageStatus.Verified;
        rec.blockHash = blockhash(block.number - 1);
        rec.validationMode = ValidationMode.PBFT;

        emit PackageVerified(key, canonical, approvals);
    }
    
    /// @notice Verify a ZK proof for an existing pending package
    /// @param canonical Package canonical ID
    /// @param proof     Groth16 proof
    /// @param publicInputs Public inputs
    function verifyZKProof(
        string calldata canonical,
        uint256[8] calldata proof,
        uint256[] calldata publicInputs
    ) external whenNotPaused {
        bytes32 key = _key(canonical);
        PackageRecord storage rec = packages[key];

        if (rec.status == PackageStatus.Unknown) revert NotFound(canonical);
        if (rec.status == PackageStatus.Verified) return; // Already verified
        if (rec.status == PackageStatus.Revoked) revert AlreadyRevoked(canonical);
        if (!zkValidationEnabled) revert ZKDisabled();

        // Verify ZK proof
        bool proofValid = zkVerifier.verifyProof(proof, publicInputs);
        if (!proofValid) revert InvalidZKProof();

        // Update record
        bytes32 proofHash = keccak256(abi.encodePacked(proof, publicInputs));
        rec.status = PackageStatus.Verified;
        rec.validationMode = ValidationMode.ZKProof;
        rec.zkProofHash = proofHash;
        rec.blockHash = blockhash(block.number - 1);

        // Store proof
        zkProofs[key] = ZKProofData({
            proof: proof,
            publicInputs: publicInputs,
            verifiedAt: uint64(block.timestamp)
        });

        emit PackageVerifiedZK(key, canonical, proofHash);
    }

    // ── Revocation ────────────────────────────────────────────────────────────

    function revokePackage(
        string calldata canonical,
        string calldata reason,
        Staking.Severity severity
    ) external whenNotPaused {
        bytes32 key = _key(canonical);
        PackageRecord storage rec = packages[key];

        if (rec.status == PackageStatus.Unknown) revert NotFound(canonical);
        if (rec.status == PackageStatus.Revoked)  revert AlreadyRevoked(canonical);

        bool isGov       = msg.sender == governance;
        bool isPublisher = msg.sender == rec.publisher;
        require(isGov || isPublisher, "Only governance or publisher may revoke");

        rec.status            = PackageStatus.Revoked;
        rec.revocationReason  = reason;

        // If governance is revoking (malicious package), slash publisher stake based on severity.
        if (isGov) {
            staking.slashSeverity(rec.publisher, severity, reason);
        }

        emit PackageRevoked(key, canonical, reason);
    }

    // ── Queries ───────────────────────────────────────────────────────────────

    function getPackage(string calldata canonical)
        external view
        returns (PackageRecord memory)
    {
        return packages[_key(canonical)];
    }

    function getStatus(string calldata canonical)
        external view
        returns (PackageStatus)
    {
        return packages[_key(canonical)].status;
    }

    function getConsensusProof(string calldata canonical)
        external view
        returns (ValidatorSig[] memory)
    {
        return consensusProofs[_key(canonical)];
    }
    
    function getZKProof(string calldata canonical)
        external view
        returns (ZKProofData memory)
    {
        return zkProofs[_key(canonical)];
    }

    // ── Governance ────────────────────────────────────────────────────────────

    function setGovernance(address newGov) external onlyGovernance {
        governance = newGov;
        emit GovernanceUpdated(newGov);
    }

    function setQuorum(uint8 pct) external onlyGovernance {
        require(pct >= 51 && pct <= 100, "Quorum must be 51-100%");
        quorumPct = pct;
        emit QuorumUpdated(pct);
    }
    
    function setZKValidationEnabled(bool enabled) external onlyGovernance {
        zkValidationEnabled = enabled;
        emit ZKValidationToggled(enabled);
    }
    
    function setZKValidationFee(uint256 fee) external onlyGovernance {
        zkValidationFee = fee;
    }
    
    /// @notice Withdraw accumulated ZK validation fees
    /// @dev Protected against reentrancy since it transfers ETH.
    function withdrawFees(address payable to, uint256 amount) external onlyGovernance nonReentrant {
        require(address(this).balance >= amount, "Insufficient balance");
        to.transfer(amount);
    }

    // ── Dependency tracking ──────────────────────────────────────────────────

    /// @notice dependentCount stores how many packages depend on the given key.
    mapping(bytes32 => uint256) public dependentCounts;

    /// @notice Record a dependency relationship (governance / cross-chain oracle).
    function setDependentCount(string calldata canonical, uint256 count) external onlyGovernance {
        dependentCounts[_key(canonical)] = count;
    }

    /// @notice Return the on-chain dependent count for a package.
    function getDependentCount(string memory canonical) external view returns (uint256) {
        return dependentCounts[keccak256(abi.encodePacked(canonical))];
    }

    // ── Internal helpers ──────────────────────────────────────────────────────

    function _key(string calldata canonical) internal pure returns (bytes32) {
        return keccak256(abi.encodePacked(canonical));
    }

    function _sigDigest(string memory canonical, bytes32 contentHash)
        internal pure returns (bytes32)
    {
        return keccak256(
            abi.encodePacked(
                "\x19Ethereum Signed Message:\n32",
                keccak256(abi.encodePacked(canonical, contentHash))
            )
        );
    }

    function _recoverSigner(bytes32 digest, bytes memory sig)
        internal pure returns (address)
    {
        require(sig.length == 65, "Invalid signature length");
        bytes32 r; bytes32 s; uint8 v;
        assembly {
            r := mload(add(sig, 32))
            s := mload(add(sig, 64))
            v := byte(0, mload(add(sig, 96)))
        }
        return ecrecover(digest, v, r, s);
    }
}
