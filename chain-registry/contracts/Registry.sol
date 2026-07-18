// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "./Staking.sol";
import "./Reputation.sol";
import "./VRF.sol";
import "./Governance.sol";
import "./ZKVerifier.sol";

interface IGovernancePause {
    function isPaused() external view returns (bool);
}

/// @title ChainRegistry
/// @notice Legacy on-chain package registry.
/// @dev ZK package qualification and ZK batch-settlement entry points are
///      retired. A proof may establish only the statement encoded in its
///      circuit; the legacy circuits do not establish package safety or a
///      registry state transition.
contract ChainRegistry {

    // ── ZK public-input binding ──────────────────────────────────────────────

    string private constant PACKAGE_ZK_BINDING_DOMAIN = "creg-zk-package-v1";
    uint256 private constant UINT128_MASK = type(uint128).max;

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
        bytes32 blockHash;         // Block hash at inclusion time (best-effort; 0x0 after 256 blocks)
        uint256 blockNumber;       // Block number — always available, unlike blockhash()
        PackageStatus status;
        string  revocationReason;
        ValidationMode validationMode;
        bytes32 zkProofHash;       // Hash of ZK proof (if ZK validated)
    }

    struct ValidatorSig {
        address validator;
        bytes   signature;         // ECDSA sig over domain + set snapshot + subject
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
    
    // Legacy validation configuration. This value remains for ABI compatibility,
    // but qualification cannot be re-enabled by this implementation.
    bool public zkValidationEnabled;
    uint256 public zkValidationFee;

    // ── L2 Rollup State ────────────────────────────────────────────────────────
    
    bytes32 public latestStateRoot;  // Merkle root of the entire verified registry
    uint256 public totalBatches;      // Historical legacy batch-submission counter
    
    /// batchId → stateRoot
    mapping(uint256 => bytes32) public batchRoots;
    
    /// batchId → transactionDataRoot (for Data Availability)
    mapping(uint256 => bytes32) public batchDataRoots;

    /// Relays (e.g. BatchOperations) that may submit packages on behalf of a staked publisher.
    mapping(address => bool) public packageSubmitRelays;

    /// Relays (e.g. chain node bridge) authorized to call finalizePackage when enforcement is on.
    mapping(address => bool) public packageFinalizeRelays;

    /// When true, finalizePackage must be called by an authorized relay (testnet hardening).
    bool public enforceFinalizeRelays;

    bytes32 public constant VALIDATOR_SIGNATURE_DOMAIN =
        keccak256("CREG-REGISTRY-VALIDATOR-SIGNATURE-V2");

    /// Per-package validator signature replay guard (canonical key → validator → signed).
    mapping(bytes32 => mapping(address => bool)) public validatorSignedForPackage;

    // ── Events ────────────────────────────────────────────────────────────────

    event PackageSubmitted(bytes32 indexed key, string canonical, address indexed publisher);
    event PackageVerified (bytes32 indexed key, string canonical, uint validatorCount);
    event PackageVerifiedZK(bytes32 indexed key, string canonical, bytes32 proofHash);
    event PackageRevoked  (bytes32 indexed key, string canonical, string reason);
    event GovernanceUpdated(address newGovernance);
    event QuorumUpdated(uint8 newQuorumPct);
    event ZKValidationToggled(bool enabled);
    event PackageSubmitRelayUpdated(address indexed relay, bool enabled);
    event PackageFinalizeRelayUpdated(address indexed relay, bool enabled);
    event EnforceFinalizeRelaysUpdated(bool enabled);
    
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
    error InvalidZKPublicInputs();
    error NotGovernance();
    error NotPublisher();
    error ZKDisabled();
    error NotAuthorizedSubmitRelay();
    error NotAuthorizedFinalizeRelay();
    error AlreadyVerified(string canonical);
    error ValidatorAlreadySigned(address validator);
    error ZKQualificationRetired();
    error LegacyCheckpointingRetired();

    // ── Modifiers ─────────────────────────────────────────────────────────────

    modifier onlyGovernance() {
        if (msg.sender != governance) revert NotGovernance();
        _;
    }

    modifier whenNotPaused() {
        if (IGovernancePause(governance).isPaused()) {
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
        // CR-02: caller-supplied ZK inputs must never qualify a package.
        zkValidationEnabled = false;
        zkValidationFee = 0;
        enforceFinalizeRelays = true; // Safer default: only allowlisted relays by default (governance can relax)
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
        _storePendingPackage(msg.sender, canonical, contentHash, ipfsCid);
    }

    /// @notice Submit a package on behalf of a staked publisher via an authorized relay.
    /// @dev Used by BatchOperations so `msg.sender` can differ from the publisher while
    ///      preserving the publisher stake check against `publisher`.
    function submitPackageFor(
        address publisher,
        string calldata canonical,
        bytes32 contentHash,
        string calldata ipfsCid
    ) external whenNotPaused {
        if (msg.sender != publisher && !packageSubmitRelays[msg.sender]) {
            revert NotAuthorizedSubmitRelay();
        }
        _storePendingPackage(publisher, canonical, contentHash, ipfsCid);
    }

    function _storePendingPackage(
        address publisher,
        string calldata canonical,
        bytes32 contentHash,
        string calldata ipfsCid
    ) internal {
        bytes32 key = _key(canonical);
        PackageRecord storage rec = packages[key];

        if (rec.status != PackageStatus.Unknown) {
            revert AlreadyExists(canonical);
        }

        // Publisher must have staked tokens to publish.
        require(staking.stakedBalance(publisher) > 0, "Publisher must stake first");
        // CR-06: reserve one minimum publisher bond for this exact release so
        // a publisher cannot publish unlimited versions and immediately
        // withdraw the only stake at risk.
        staking.reservePublisherLiability(key, publisher, staking.minPublisherStake());

        packages[key] = PackageRecord({
            canonical:         canonical,
            contentHash:       contentHash,
            ipfsCid:           ipfsCid,
            publisher:         publisher,
            publishedAt:       uint64(block.timestamp),
            // Note: blockhash() only works for the 256 most recent blocks.
            // blockNumber is the reliable inclusion anchor; blockHash is best-effort.
            blockHash:         blockhash(block.number - 1),
            blockNumber:       block.number,
            status:            PackageStatus.Pending,
            revocationReason:  "",
            validationMode:    ValidationMode.PBFT,
            zkProofHash:       bytes32(0)
        });

        emit PackageSubmitted(key, canonical, publisher);
    }
    
    /// @notice Retired: ZK proofs cannot qualify a package.
    /// @dev Retained only to preserve the deployed ABI during migration.
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
        revert ZKQualificationRetired();
    }

    // ── Retired batch-settlement path ────────────────────────────────────────

    /// @notice Retired: the legacy batch circuit did not prove a state transition.
    /// @dev Retained only to preserve the deployed ABI during migration.
    /// @param prevRoot Previous state root (must match latestStateRoot)
    /// @param nextRoot New state root after processing the batch
    /// @param txCount  Number of verified packages in this batch
    /// @param dataRoot Merkle root of the transaction data (for DA)
    /// @param proof    Unused legacy argument.
    function submitRollupBatch(
        bytes32 prevRoot,
        bytes32 nextRoot,
        uint256 txCount,
        bytes32 dataRoot,
        uint256[8] calldata proof,
        uint256[] calldata publicInputs
    ) external onlyGovernance {
        revert LegacyCheckpointingRetired();
    }

    // ── Consensus-facing ─────────────────────────────────────────────────────

    /// @notice Finalize a package after PBFT consensus.
    /// @dev Called by the chain node once the off-chain PBFT round completes.
    ///      Signature replay per validator is rejected. When `enforceFinalizeRelays`
    ///      is enabled (governance), only authorized relay contracts may call.
    function finalizePackage(
        string calldata canonical,
        ValidatorSig[] calldata sigs
    ) external whenNotPaused {
        if (enforceFinalizeRelays && !packageFinalizeRelays[msg.sender]) {
            revert NotAuthorizedFinalizeRelay();
        }

        bytes32 key = _key(canonical);
        PackageRecord storage rec = packages[key];

        if (rec.status == PackageStatus.Unknown) revert NotFound(canonical);
        if (rec.status == PackageStatus.Verified) revert AlreadyVerified(canonical);
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

            if (validatorSignedForPackage[key][s.validator]) {
                revert ValidatorAlreadySigned(s.validator);
            }

            // Verify a chain/contract/set-domain signature over the exact
            // package subject. This prevents cross-deployment replay.
            address recovered = _recoverSigner(digest, s.signature);
            if (recovered != s.validator) revert InvalidSignature(s.validator);

            validatorSignedForPackage[key][s.validator] = true;
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
        rec.blockNumber = block.number;
        rec.validationMode = ValidationMode.PBFT;
        // Keep the publisher bond reserved through the challenge window.
        try staking.markPackageFinalized(key) {} catch {}

        emit PackageVerified(key, canonical, approvals);
    }
    
    /// @notice Retired: ZK proofs cannot qualify an existing pending package.
    /// @dev Retained only to preserve the deployed ABI during migration.
    /// @param canonical Package canonical ID
    /// @param proof     Groth16 proof
    /// @param publicInputs Public inputs
    function verifyZKProof(
        string calldata canonical,
        uint256[8] calldata proof,
        uint256[] calldata publicInputs
    ) external whenNotPaused {
        revert ZKQualificationRetired();
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

        // Release the package-specific liability. Governance revocations slash
        // the reserved bond; publisher-initiated revocations release it.
        try staking.resolvePublisherLiability(key, isGov, reason) {} catch {}
        // Apply any additional severity slash after resolving the reserved
        // release bond, so the two accounting paths cannot double-spend or
        // underflow the reservation.
        if (isGov) {
            staking.slashSeverity(rec.publisher, severity, reason);
        }

        emit PackageRevoked(key, canonical, reason);
    }

    /// @notice Narrow emergency revocation path available while governance has
    /// paused the system.  It cannot publish, verify, transfer funds, or alter
    /// policy; it only records a fail-closed package state and applies the
    /// normal governance slash rule.
    function emergencyRevokePackage(
        string calldata canonical,
        string calldata reason,
        Staking.Severity severity
    ) external onlyGovernance {
        bytes32 key = _key(canonical);
        PackageRecord storage rec = packages[key];
        if (rec.status == PackageStatus.Unknown) revert NotFound(canonical);
        if (rec.status == PackageStatus.Revoked) revert AlreadyRevoked(canonical);

        rec.status = PackageStatus.Revoked;
        rec.revocationReason = reason;
        try staking.resolvePublisherLiability(key, true, reason) {} catch {}
        staking.slashSeverity(rec.publisher, severity, reason);
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

    /// @notice Return the required first two public inputs for package ZK proofs.
    /// @dev Circuits must expose and constrain these values as public signals at indexes 0 and 1.
    function packageZKBindingInputs(
        string calldata canonical,
        bytes32 contentHash,
        string calldata ipfsCid
    ) external pure returns (uint256 bindingHigh, uint256 bindingLow) {
        return _packageZKBindingInputs(canonical, contentHash, ipfsCid);
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
        if (enabled) revert ZKQualificationRetired();
        zkValidationEnabled = enabled;
        emit ZKValidationToggled(enabled);
    }
    
    function setZKValidationFee(uint256 fee) external onlyGovernance {
        zkValidationFee = fee;
    }

    /// @notice Allow or revoke a relay contract that submits packages for staked publishers.
    function setPackageSubmitRelay(address relay, bool enabled) external onlyGovernance {
        packageSubmitRelays[relay] = enabled;
        emit PackageSubmitRelayUpdated(relay, enabled);
    }

    /// @notice Allow or revoke a relay that may call finalizePackage when enforcement is on.
    function setPackageFinalizeRelay(address relay, bool enabled) external onlyGovernance {
        packageFinalizeRelays[relay] = enabled;
        emit PackageFinalizeRelayUpdated(relay, enabled);
    }

    /// @notice Toggle finalizePackage relay allowlist (off by default — permissionless relaying).
    function setEnforceFinalizeRelays(bool enabled) external onlyGovernance {
        enforceFinalizeRelays = enabled;
        emit EnforceFinalizeRelaysUpdated(enabled);
    }
    
    /// @notice Withdraw legacy fees accrued before the ZK qualification freeze.
    /// @dev Protected against reentrancy since it transfers ETH.
    function withdrawFees(address payable to, uint256 amount) external onlyGovernance nonReentrant {
        require(address(this).balance >= amount, "Insufficient balance");
        (bool success,) = to.call{value: amount}("");
        require(success, "ETH transfer failed");
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
        internal view returns (bytes32)
    {
        return keccak256(
            abi.encodePacked(
                "\x19Ethereum Signed Message:\n32",
                keccak256(
                    abi.encode(
                        VALIDATOR_SIGNATURE_DOMAIN,
                        block.chainid,
                        address(this),
                        staking.activeValidatorCount(),
                        quorumPct,
                        canonical,
                        contentHash
                    )
                )
            )
        );
    }

    function _assertPackageProofBinding(
        string memory canonical,
        bytes32 contentHash,
        string memory ipfsCid,
        uint256[] calldata publicInputs
    ) internal pure {
        if (publicInputs.length < 2) revert InvalidZKPublicInputs();

        // To address DEEP_DIVE ISSUE-001 (ZK must bind actual content/manifest hashes):
        // The circuit binds content_hash and manifest_hash as public inputs (limbs).
        // We now bind using the contentHash limbs directly (what the proof commits to).
        // This ensures the submitted proof is for *this* contentHash (circuit-enforced).
        // (ipfsCid + canonical provide additional context in submit; full tuple binding can use separate check if needed.)
        (uint256 contentHigh, uint256 contentLow) = _contentHashToLimbs(contentHash);
        if (publicInputs[0] != contentLow || publicInputs[1] != contentHigh) {
            revert InvalidZKPublicInputs();
        }
        // Note: manifest limbs would be publicInputs[2,3] from circuit; can extend assert here if manifest provided in ZK submit.
    }

    function _packageZKBindingInputs(
        string memory canonical,
        bytes32 contentHash,
        string memory ipfsCid
    ) internal pure returns (uint256 bindingHigh, uint256 bindingLow) {
        bytes32 bindingHash = keccak256(
            abi.encode(PACKAGE_ZK_BINDING_DOMAIN, canonical, contentHash, ipfsCid)
        );
        uint256 binding = uint256(bindingHash);
        bindingHigh = binding >> 128;
        bindingLow = binding & UINT128_MASK;
    }

    function _contentHashToLimbs(bytes32 contentHash)
        internal pure returns (uint256 high, uint256 low)
    {
        // Split 32-byte hash into high/low 128-bit limbs (matching circuit bind_hash32_limbs and public inputs order).
        uint256 h = uint256(contentHash);
        high = h >> 128;
        low = h & UINT128_MASK;
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
        if (v < 27) v += 27;
        // Reject high-s signatures (EIP-2 malleability), matching the admission
        // path in Staking._recoverSigner. ecrecover accepts both s and (n - s),
        // so without this a third party could mint a second valid signature for
        // the same payload. Returns address(0) on a malleable signature, which
        // callers already treat as a non-active / invalid signer.
        if (uint256(s) > 0x7FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF5D576E7357A4501DDFE92F46681B20A0) {
            return address(0);
        }
        return ecrecover(digest, v, r, s);
    }
}
