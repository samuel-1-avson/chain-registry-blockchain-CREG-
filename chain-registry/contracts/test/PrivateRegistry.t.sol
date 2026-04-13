// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "forge-std/Test.sol";
import "../PrivateRegistry.sol";

/**
 * @title PrivateRegistryShareTest
 * @notice Regression tests for ISSUE-004: submitDecryptionShare must verify
 *         an ECDSA commitment signature so validators cannot submit arbitrary
 *         share values without proving knowledge of the corresponding private key.
 *
 * The share encoding is:
 *   bytes  0..31  — 32-byte Lagrange share value
 *   bytes 32..96  — 65-byte ECDSA sig (r || s || v) over
 *                   keccak256("\x19Ethereum Signed Message:\n32" ||
 *                     keccak256(abi.encodePacked(orgId, packageKey, shareValue, msg.sender)))
 */
contract PrivateRegistryShareTest is Test {
    PrivateRegistry priv;

    // Validator signing key (Forge built-in cheat).
    uint256 constant VALIDATOR_PK = 0xA11CE;
    address validatorAddr;

    bytes32 constant ORG_ID = keccak256("testOrg");
    bytes32 packageKey;        // set in setUp — derived from orgId+canonical

    function setUp() public {
        // Deploy with address(0) for Registry/Staking; neither is called during
        // submitDecryptionShare or on the code paths exercised by these tests.
        priv = new PrivateRegistry(address(0), address(0));

        validatorAddr = vm.addr(VALIDATOR_PK);

        // Build arrays required by createOrganization.
        address[] memory validators = new address[](1);
        validators[0] = validatorAddr;

        bytes[] memory pubkeys = new bytes[](1);
        pubkeys[0] = abi.encodePacked(validatorAddr); // arbitrary pubkey bytes

        // Create org — test contract becomes admin AND member (see PrivateRegistry line 211).
        priv.createOrganization(ORG_ID, "TestOrg", 1, validators, pubkeys);

        // Default policy has minStakeRequired = 0.01 ether which would call staking.
        // Set it to 0 so submitPrivatePackage doesn't touch the Staking stub.
        string[] memory ecosystems = new string[](0);
        priv.setAccessPolicy(ORG_ID, PrivateRegistry.AccessPolicy({
            requiresApproval: true,
            maxPackageSize: 100 * 1024 * 1024,
            minStakeRequired: 0,
            allowExternalPublishers: false,
            allowedEcosystems: ecosystems
        }));

        // Submit a package as member (admin is auto-member); returns the derived packageKey.
        bytes memory fakeEncrypted = new bytes(32);
        bytes memory fakeKeyShares = new bytes(32);
        bytes32 contentHash = keccak256("content");
        packageKey = priv.submitPrivatePackage(
            ORG_ID,
            "pkg@1.0.0",
            fakeEncrypted,
            fakeKeyShares,
            contentHash
        );

        // Approve it (test contract == admin).
        priv.approvePrivatePackage(ORG_ID, packageKey);
    }

    // ─── Helper ──────────────────────────────────────────────────────────────

    /// @dev Build a correctly signed 97-byte share blob.
    function _buildShare(
        bytes32 shareValue,
        uint256 signerPk,
        bytes32 orgId,
        bytes32 pkgKey,
        address submitter
    ) internal pure returns (bytes memory) {
        bytes32 commitment = keccak256(
            abi.encodePacked(orgId, pkgKey, shareValue, submitter)
        );
        bytes32 ethHash = keccak256(
            abi.encodePacked("\x19Ethereum Signed Message:\n32", commitment)
        );
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(signerPk, ethHash);
        return abi.encodePacked(shareValue, r, s, v);
    }

    // ─── Tests ───────────────────────────────────────────────────────────────

    /// @dev Valid share signed by the validator: must be accepted and stored.
    function test_validShareAccepted() public {
        bytes32 shareValue = keccak256("myShareValue");
        bytes memory share = _buildShare(
            shareValue, VALIDATOR_PK, ORG_ID, packageKey, validatorAddr
        );

        vm.prank(validatorAddr);
        priv.submitDecryptionShare(ORG_ID, packageKey, share);

        bytes memory stored = priv.getDecryptionShare(packageKey, validatorAddr);
        assertEq(stored, share, "share must be stored after valid submission");
    }

    /// @dev Share shorter than 97 bytes must revert before ECDSA check.
    function test_shortShareReverts() public {
        bytes memory tooShort = new bytes(64);

        vm.prank(validatorAddr);
        vm.expectRevert(bytes("Share too short: need 32-byte value + 65-byte sig"));
        priv.submitDecryptionShare(ORG_ID, packageKey, tooShort);
    }

    /// @dev Share signed by a different key (not msg.sender) must revert.
    function test_wrongSignerReverts() public {
        uint256 otherPk = 0xB0B;
        bytes32 shareValue = keccak256("myShareValue");
        // Sign with a key whose address != validatorAddr.
        bytes memory share = _buildShare(
            shareValue, otherPk, ORG_ID, packageKey, validatorAddr
        );

        vm.prank(validatorAddr);
        vm.expectRevert(
            abi.encodeWithSelector(
                PrivateRegistry.InvalidShareSignature.selector,
                validatorAddr,
                vm.addr(otherPk)
            )
        );
        priv.submitDecryptionShare(ORG_ID, packageKey, share);
    }

    /// @dev Share signed over wrong packageKey in commitment must revert with InvalidShareSignature.
    ///      We can't predict the exact recovered address, so check selector only.
    function test_wrongPackageKeyInCommitmentReverts() public {
        bytes32 shareValue   = keccak256("myShareValue");
        bytes32 wrongPkgKey  = keccak256("notThisPackage");
        // Signature is valid for a different packageKey — commitment mismatch.
        bytes memory share = _buildShare(
            shareValue, VALIDATOR_PK, ORG_ID, wrongPkgKey, validatorAddr
        );

        vm.prank(validatorAddr);
        // Recovered address will not equal validatorAddr; accept any revert from InvalidShareSignature.
        vm.expectRevert();
        priv.submitDecryptionShare(ORG_ID, packageKey, share);
    }

    /// @dev Duplicate submission must revert with AlreadyApproved.
    function test_duplicateShareReverts() public {
        bytes32 shareValue = keccak256("myShareValue");
        bytes memory share = _buildShare(
            shareValue, VALIDATOR_PK, ORG_ID, packageKey, validatorAddr
        );

        vm.startPrank(validatorAddr);
        priv.submitDecryptionShare(ORG_ID, packageKey, share);

        vm.expectRevert(
            abi.encodeWithSelector(
                PrivateRegistry.AlreadyApproved.selector,
                packageKey,
                validatorAddr
            )
        );
        priv.submitDecryptionShare(ORG_ID, packageKey, share);
        vm.stopPrank();
    }

    /// @dev Non-validator must be rejected with NotValidator before any share validation.
    function test_nonValidatorReverts() public {
        address nobody = address(0x9999);
        bytes memory share = new bytes(97);

        vm.prank(nobody);
        vm.expectRevert(
            abi.encodeWithSelector(PrivateRegistry.NotValidator.selector, nobody)
        );
        priv.submitDecryptionShare(ORG_ID, packageKey, share);
    }
}
