// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "./Registry.sol";
import "./Staking.sol";

/// @title PrivateRegistry
/// @notice Enterprise-grade private package registries with threshold encryption
/// @dev Allows organizations to create private registries where packages are
///      encrypted and only decryptable by a quorum of validators.
contract PrivateRegistry {
    
    // ── Structs ───────────────────────────────────────────────────────────────
    
    struct Organization {
        bytes32 orgId;
        address admin;
        string name;
        uint256 createdAt;
        bool active;
        uint8 threshold;           // Minimum validators needed (M-of-N)
        uint256 validatorCount;
        mapping(address => bool) isValidator;
        mapping(address => bytes) validatorPublicKeys;
        address[] validatorList;
    }
    
    struct PrivatePackage {
        bytes32 orgId;
        string canonical;
        bytes encryptedContent;    // AES-256-GCM encrypted package
        bytes encryptedKeyShares;  // Encrypted shares for each validator
        bytes32 contentHash;       // Hash of decrypted content
        address publisher;
        uint256 publishedAt;
        uint8 approvals;           // Current approval count
        mapping(address => bool) hasApproved;
        PackageStatus status;
    }
    
    struct AccessPolicy {
        bool requiresApproval;     // Admin approval for each package
        uint256 maxPackageSize;    // Maximum package size in bytes
        uint256 minStakeRequired;  // Minimum stake for publishers
        bool allowExternalPublishers;
        string[] allowedEcosystems;
    }
    
    enum PackageStatus { Pending, Approved, Rejected, Revoked }
    
    // ── Storage ───────────────────────────────────────────────────────────────
    
    /// orgId → Organization
    mapping(bytes32 => Organization) public organizations;
    
    /// packageKey → PrivatePackage
    mapping(bytes32 => PrivatePackage) public privatePackages;
    
    /// orgId → AccessPolicy
    mapping(bytes32 => AccessPolicy) public policies;
    
    /// orgId → member address → isMember
    mapping(bytes32 => mapping(address => bool)) public orgMembers;
    
    /// orgId → pending invitations
    mapping(bytes32 => mapping(address => uint256)) public pendingInvitations;

    /// orgId → list of package keys (for key rotation tracking)
    mapping(bytes32 => bytes32[]) internal orgPackageKeys;

    /// orgId → packageKey → true if key shares need rotation after a validator removal
    mapping(bytes32 => mapping(bytes32 => bool)) public pendingKeyRotation;

    /// packageKey → validator address → submitted decryption share
    mapping(bytes32 => mapping(address => bytes)) public decryptionShares;
    
    ChainRegistry public immutable publicRegistry;
    Staking public immutable staking;
    
    uint256 public orgCount;
    uint256 public constant MAX_VALIDATORS = 50;
    uint256 public constant MAX_PACKAGE_SIZE = 100 * 1024 * 1024; // 100MB
    
    // ── Events ────────────────────────────────────────────────────────────────
    
    event OrganizationCreated(
        bytes32 indexed orgId,
        string name,
        address indexed admin,
        uint8 threshold
    );
    event ValidatorAdded(bytes32 indexed orgId, address indexed validator);
    event ValidatorRemoved(bytes32 indexed orgId, address indexed validator);
    event PrivatePackageSubmitted(
        bytes32 indexed orgId,
        bytes32 indexed packageKey,
        string canonical
    );
    event PrivatePackageApproved(
        bytes32 indexed orgId,
        bytes32 indexed packageKey,
        uint8 approvalCount
    );
    event KeyShareSubmitted(
        bytes32 indexed orgId,
        bytes32 indexed packageKey,
        address indexed validator
    );
    event OrganizationDeactivated(bytes32 indexed orgId);
    event AccessPolicyUpdated(bytes32 indexed orgId);
    event KeyRotationRequired(
        bytes32 indexed orgId,
        address indexed removedValidator,
        uint256 affectedPackageCount
    );
    event PackageKeySharesRotated(
        bytes32 indexed orgId,
        bytes32 indexed packageKey,
        address indexed submitter
    );
    
    // ── Errors ────────────────────────────────────────────────────────────────
    
    error Unauthorized();
    error OrganizationNotFound(bytes32 orgId);
    error OrganizationExists(bytes32 orgId);
    error ValidatorNotFound(address validator);
    error ValidatorAlreadyExists(address validator);
    error PackageNotFound(bytes32 packageKey);
    error PackageTooLarge(uint256 size, uint256 max);
    error ThresholdTooHigh(uint8 threshold, uint256 validators);
    error AlreadyApproved(bytes32 packageKey, address validator);
    error InvalidThreshold(uint8 threshold);
    error NotValidator(address addr);
    error NotOrganizationMember(bytes32 orgId, address member);
    error InvalidShareSignature(address expected, address recovered);
    
    // ── Modifiers ─────────────────────────────────────────────────────────────
    
    modifier onlyOrgAdmin(bytes32 orgId) {
        if (organizations[orgId].admin != msg.sender) revert Unauthorized();
        _;
    }
    
    modifier onlyOrgValidator(bytes32 orgId) {
        if (!organizations[orgId].isValidator[msg.sender]) revert NotValidator(msg.sender);
        _;
    }
    
    modifier onlyOrgMember(bytes32 orgId) {
        if (!orgMembers[orgId][msg.sender]) revert NotOrganizationMember(orgId, msg.sender);
        _;
    }
    
    modifier orgExists(bytes32 orgId) {
        if (!organizations[orgId].active) revert OrganizationNotFound(orgId);
        _;
    }
    
    // ── Constructor ───────────────────────────────────────────────────────────
    
    constructor(address _publicRegistry, address _staking) {
        publicRegistry = ChainRegistry(_publicRegistry);
        staking = Staking(_staking);
    }
    
    // ── Organization Management ───────────────────────────────────────────────
    
    /// @notice Create a new private organization registry
    /// @param orgId Unique organization identifier (bytes32 hash)
    /// @param name Human-readable organization name
    /// @param threshold Minimum validators needed for decryption (M-of-N)
    /// @param validators Initial validator addresses
    /// @param validatorPubKeys Public keys for each validator (for encryption)
    function createOrganization(
        bytes32 orgId,
        string calldata name,
        uint8 threshold,
        address[] calldata validators,
        bytes[] calldata validatorPubKeys
    ) external {
        if (organizations[orgId].active) revert OrganizationExists(orgId);
        if (validators.length == 0 || validators.length > MAX_VALIDATORS) {
            revert InvalidThreshold(0);
        }
        if (threshold == 0 || threshold > validators.length) {
            revert InvalidThreshold(threshold);
        }
        if (validators.length != validatorPubKeys.length) {
            revert("Validator/pubkey count mismatch");
        }
        
        Organization storage org = organizations[orgId];
        org.orgId = orgId;
        org.admin = msg.sender;
        org.name = name;
        org.createdAt = block.timestamp;
        org.active = true;
        org.threshold = threshold;
        org.validatorCount = validators.length;
        
        // Add validators
        for (uint i = 0; i < validators.length; i++) {
            address validator = validators[i];
            org.isValidator[validator] = true;
            org.validatorPublicKeys[validator] = validatorPubKeys[i];
            org.validatorList.push(validator);
        }
        
        // Admin is also a member
        orgMembers[orgId][msg.sender] = true;
        
        // Set default access policy
        policies[orgId] = AccessPolicy({
            requiresApproval: true,
            maxPackageSize: MAX_PACKAGE_SIZE,
            minStakeRequired: 0.01 ether,
            allowExternalPublishers: false,
            allowedEcosystems: new string[](0)
        });
        
        orgCount++;
        
        emit OrganizationCreated(orgId, name, msg.sender, threshold);
    }
    
    /// @notice Add a validator to an organization
    function addValidator(
        bytes32 orgId,
        address validator,
        bytes calldata publicKey
    ) external onlyOrgAdmin(orgId) orgExists(orgId) {
        Organization storage org = organizations[orgId];
        
        if (org.isValidator[validator]) revert ValidatorAlreadyExists(validator);
        if (org.validatorCount >= MAX_VALIDATORS) revert("Max validators reached");
        
        org.isValidator[validator] = true;
        org.validatorPublicKeys[validator] = publicKey;
        org.validatorList.push(validator);
        org.validatorCount++;
        
        emit ValidatorAdded(orgId, validator);
    }
    
    /// @notice Remove a validator from an organization
    /// @dev Flags all existing packages for key rotation; the admin must call
    ///      `rotatePackageKeyShares()` on each to re-encrypt with the new validator set.
    function removeValidator(
        bytes32 orgId,
        address validator
    ) external onlyOrgAdmin(orgId) orgExists(orgId) {
        Organization storage org = organizations[orgId];
        
        if (!org.isValidator[validator]) revert ValidatorNotFound(validator);
        if (org.validatorCount - 1 < org.threshold) {
            revert ThresholdTooHigh(org.threshold, org.validatorCount - 1);
        }
        
        org.isValidator[validator] = false;
        delete org.validatorPublicKeys[validator];
        org.validatorCount--;
        
        // Note: We don't remove from validatorList to maintain index stability

        // Flag all existing packages as needing key share rotation.
        bytes32[] storage pkgKeys = orgPackageKeys[orgId];
        uint256 affected = 0;
        for (uint256 i = 0; i < pkgKeys.length; i++) {
            if (privatePackages[pkgKeys[i]].status == PackageStatus.Approved) {
                pendingKeyRotation[orgId][pkgKeys[i]] = true;
                affected++;
            }
        }
        
        emit ValidatorRemoved(orgId, validator);
        if (affected > 0) {
            emit KeyRotationRequired(orgId, validator, affected);
        }
    }

    /// @notice Re-encrypt key shares for a package after validator set change.
    /// @param orgId The organization ID
    /// @param packageKey The package key to rotate shares for
    /// @param newEncryptedKeyShares New threshold-encrypted key shares for current validator set
    function rotatePackageKeyShares(
        bytes32 orgId,
        bytes32 packageKey,
        bytes calldata newEncryptedKeyShares
    ) external onlyOrgAdmin(orgId) orgExists(orgId) {
        if (!pendingKeyRotation[orgId][packageKey]) revert PackageNotFound(packageKey);

        PrivatePackage storage pkg = privatePackages[packageKey];
        require(pkg.orgId == orgId, "Package does not belong to this org");

        pkg.encryptedKeyShares = newEncryptedKeyShares;
        pendingKeyRotation[orgId][packageKey] = false;

        emit PackageKeySharesRotated(orgId, packageKey, msg.sender);
    }
    
    /// @notice Deactivate an organization
    function deactivateOrganization(bytes32 orgId) external onlyOrgAdmin(orgId) {
        organizations[orgId].active = false;
        emit OrganizationDeactivated(orgId);
    }
    
    // ── Package Publishing ────────────────────────────────────────────────────
    
    /// @notice Submit a private package with encrypted content
    /// @param orgId Organization ID
    /// @param canonical Package canonical ID (e.g., "npm:internal-lib@1.0.0")
    /// @param encryptedContent AES-256-GCM encrypted package content
    /// @param encryptedKeyShares Threshold-encrypted key shares for validators
    /// @param contentHash Hash of the decrypted content (for verification)
    function submitPrivatePackage(
        bytes32 orgId,
        string calldata canonical,
        bytes calldata encryptedContent,
        bytes calldata encryptedKeyShares,
        bytes32 contentHash
    ) external orgExists(orgId) returns (bytes32 packageKey) {
        AccessPolicy storage policy = policies[orgId];
        
        // Check authorization
        if (!policy.allowExternalPublishers && !orgMembers[orgId][msg.sender]) {
            revert NotOrganizationMember(orgId, msg.sender);
        }
        
        // Check package size
        if (encryptedContent.length > policy.maxPackageSize) {
            revert PackageTooLarge(encryptedContent.length, policy.maxPackageSize);
        }
        
        // Check publisher stake if required
        if (policy.minStakeRequired > 0) {
            require(
                staking.stakedBalance(msg.sender) >= policy.minStakeRequired,
                "Insufficient stake"
            );
        }
        
        packageKey = keccak256(abi.encodePacked(orgId, canonical));
        
        PrivatePackage storage pkg = privatePackages[packageKey];
        pkg.orgId = orgId;
        pkg.canonical = canonical;
        pkg.encryptedContent = encryptedContent;
        pkg.encryptedKeyShares = encryptedKeyShares;
        pkg.contentHash = contentHash;
        pkg.publisher = msg.sender;
        pkg.publishedAt = block.timestamp;
        pkg.status = policy.requiresApproval ? PackageStatus.Pending : PackageStatus.Approved;

        // Track package for key-rotation bookkeeping.
        orgPackageKeys[orgId].push(packageKey);
        
        emit PrivatePackageSubmitted(orgId, packageKey, canonical);
        
        // If no approval required, emit approval event
        if (!policy.requiresApproval) {
            emit PrivatePackageApproved(orgId, packageKey, 0);
        }
        
        return packageKey;
    }
    
    /// @notice Approve a pending private package (admin only)
    function approvePrivatePackage(
        bytes32 orgId,
        bytes32 packageKey
    ) external onlyOrgAdmin(orgId) orgExists(orgId) {
        PrivatePackage storage pkg = privatePackages[packageKey];
        
        if (pkg.status != PackageStatus.Pending) revert("Package not pending");
        if (pkg.orgId != orgId) revert("Package not in organization");
        
        pkg.status = PackageStatus.Approved;
        
        emit PrivatePackageApproved(orgId, packageKey, 0);
    }
    
    /// @notice Submit decryption share for a package (validator only)
    /// @dev Part of threshold decryption — validators submit their shares.
    ///
    ///      Share encoding (97 bytes total):
    ///        bytes  0..31  — 32-byte Lagrange share value
    ///        bytes 32..96  — 65-byte ECDSA signature (r || s || v) over:
    ///                        keccak256(abi.encodePacked(orgId, packageKey, shareValue, msg.sender))
    ///
    ///      This ECDSA commitment binds the share value to the specific package
    ///      and to msg.sender's identity. A validator cannot replay another
    ///      validator's share or substitute an arbitrary value without having
    ///      the corresponding private key. The recovered signer must equal
    ///      msg.sender; any mismatch reverts with InvalidShareSignature.
    function submitDecryptionShare(
        bytes32 orgId,
        bytes32 packageKey,
        bytes calldata share
    ) external onlyOrgValidator(orgId) orgExists(orgId) {
        PrivatePackage storage pkg = privatePackages[packageKey];

        if (pkg.orgId != orgId) revert("Package not in organization");
        if (pkg.status != PackageStatus.Approved) revert("Package not approved");
        if (pkg.hasApproved[msg.sender]) revert AlreadyApproved(packageKey, msg.sender);

        // Share must be exactly 32-byte value + 65-byte ECDSA signature.
        require(share.length >= 97, "Share too short: need 32-byte value + 65-byte sig");

        // Verify the submitter has a registered public key in the org.
        bytes memory validatorPubKey = organizations[orgId].validatorPublicKeys[msg.sender];
        require(validatorPubKey.length > 0, "Validator has no registered public key");

        // Parse share components.
        bytes32 shareValue;
        bytes32 sigR;
        bytes32 sigS;
        uint8   sigV;
        assembly {
            // share is calldata; share.offset points to the raw bytes
            shareValue := calldataload(share.offset)
            sigR       := calldataload(add(share.offset, 32))
            sigS       := calldataload(add(share.offset, 64))
            // v is the first byte after s (byte 96)
            sigV := byte(0, calldataload(add(share.offset, 96)))
        }
        // Ethereum ecrecover expects v in {27, 28}.
        if (sigV < 27) sigV += 27;

        // Commitment: binds shareValue to this exact (orgId, packageKey, submitter) tuple.
        bytes32 commitment = keccak256(
            abi.encodePacked(orgId, packageKey, shareValue, msg.sender)
        );
        bytes32 ethHash = keccak256(
            abi.encodePacked("\x19Ethereum Signed Message:\n32", commitment)
        );

        address recovered = ecrecover(ethHash, sigV, sigR, sigS);
        if (recovered == address(0) || recovered != msg.sender) {
            revert InvalidShareSignature(msg.sender, recovered);
        }

        // Store the share on-chain so it can be retrieved for reconstruction.
        decryptionShares[packageKey][msg.sender] = share;

        pkg.hasApproved[msg.sender] = true;
        pkg.approvals++;

        emit KeyShareSubmitted(orgId, packageKey, msg.sender);
    }

    /// @notice Retrieve a decryption share submitted by a specific validator.
    function getDecryptionShare(bytes32 packageKey, address validator)
        external
        view
        returns (bytes memory)
    {
        return decryptionShares[packageKey][validator];
    }
    
    // ── Access Control ────────────────────────────────────────────────────────
    
    /// @notice Add a member to an organization
    function addMember(bytes32 orgId, address member) external onlyOrgAdmin(orgId) {
        orgMembers[orgId][member] = true;
    }
    
    /// @notice Remove a member from an organization
    function removeMember(bytes32 orgId, address member) external onlyOrgAdmin(orgId) {
        orgMembers[orgId][member] = false;
    }
    
    /// @notice Update access policy for an organization
    function setAccessPolicy(
        bytes32 orgId,
        AccessPolicy calldata policy
    ) external onlyOrgAdmin(orgId) {
        policies[orgId] = policy;
        emit AccessPolicyUpdated(orgId);
    }
    
    // ── Queries ───────────────────────────────────────────────────────────────
    
    function getOrganization(bytes32 orgId)
        external view
        returns (
            string memory name,
            address admin,
            uint256 createdAt,
            bool active,
            uint8 threshold,
            uint256 validatorCount
        )
    {
        Organization storage org = organizations[orgId];
        return (
            org.name,
            org.admin,
            org.createdAt,
            org.active,
            org.threshold,
            org.validatorCount
        );
    }
    
    function getValidators(bytes32 orgId)
        external view
        returns (address[] memory)
    {
        return organizations[orgId].validatorList;
    }
    
    function getValidatorPublicKey(bytes32 orgId, address validator)
        external view
        returns (bytes memory)
    {
        return organizations[orgId].validatorPublicKeys[validator];
    }
    
    function canDecrypt(
        bytes32 orgId,
        bytes32 packageKey
    ) external view returns (bool) {
        PrivatePackage storage pkg = privatePackages[packageKey];
        Organization storage org = organizations[orgId];
        
        if (pkg.orgId != orgId) return false;
        if (pkg.status != PackageStatus.Approved) return false;
        if (pkg.approvals < org.threshold) return false;
        
        return true;
    }
    
    // ── Batch Operations ──────────────────────────────────────────────────────
    
    /// @notice Batch approve packages (admin only)
    function batchApprove(
        bytes32 orgId,
        bytes32[] calldata packageKeys
    ) external onlyOrgAdmin(orgId) {
        for (uint i = 0; i < packageKeys.length; i++) {
            PrivatePackage storage pkg = privatePackages[packageKeys[i]];
            if (pkg.status == PackageStatus.Pending && pkg.orgId == orgId) {
                pkg.status = PackageStatus.Approved;
                emit PrivatePackageApproved(orgId, packageKeys[i], 0);
            }
        }
    }
}
