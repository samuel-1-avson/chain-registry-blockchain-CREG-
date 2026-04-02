// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "forge-std/Test.sol";
import "../Registry.sol";
import "../Staking.sol";
import "../Reputation.sol";
import "../VRF.sol";
import "../Governance.sol";

/// @notice Full integration tests for the chain registry contracts.
contract RegistryTest is Test {

    ChainRegistry registry;
    Staking    staking;
    Reputation reputation;
    VRF        vrf;
    Governance governance;

    address alice   = makeAddr("alice");   // publisher
    address bob     = makeAddr("bob");     // validator
    address carol   = makeAddr("carol");   // validator
    address dave    = makeAddr("dave");    // governance signer

    uint256 aliceKey  = uint256(keccak256("alice-key"));
    uint256 bobKey    = uint256(keccak256("bob-key"));
    uint256 carolKey  = uint256(keccak256("carol-key"));

    string constant CANONICAL = "npm:express@4.18.2";
    bytes32 constant CONTENT_HASH = keccak256("tarball-bytes");
    string constant IPFS_CID = "bafybeigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzdi";

    function setUp() public {
        // Deploy governance with 2-of-3 multisig.
        address[] memory signers = new address[](3);
        signers[0] = alice; signers[1] = bob; signers[2] = dave;
        governance = new Governance(signers, 2);

        staking    = new Staking(address(governance));
        reputation = new Reputation(address(governance));
        vrf        = new VRF(address(governance));

        registry = new ChainRegistry(
            address(staking),
            address(reputation),
            address(vrf),
            address(governance),
            address(0)
        );

        staking.setRegistry(address(registry));
        reputation.setRegistry(address(registry));

        // Fund accounts.
        vm.deal(alice, 10 ether);
        vm.deal(bob,   10 ether);
        vm.deal(carol, 10 ether);
    }

    // ── Publisher staking ─────────────────────────────────────────────────────

    function test_publisherMustStakeToSubmit() public {
        vm.prank(alice);
        vm.expectRevert("Publisher must stake first");
        registry.submitPackage(CANONICAL, CONTENT_HASH, IPFS_CID);
    }

    function test_publisherCanStakeAndSubmit() public {
        _stakeAsPublisher(alice);
        _submitPackage(alice);

        Registry.PackageRecord memory rec = registry.getPackage(CANONICAL);
        assertEq(rec.canonical, CANONICAL);
        assertEq(uint(rec.status), uint(Registry.PackageStatus.Pending));
        assertEq(rec.publisher, alice);
    }

    function test_cannotSubmitDuplicate() public {
        _stakeAsPublisher(alice);
        _submitPackage(alice);

        vm.prank(alice);
        vm.expectRevert();
        registry.submitPackage(CANONICAL, CONTENT_HASH, IPFS_CID);
    }

    // ── Consensus finalization ────────────────────────────────────────────────

    function test_finalizeRequiresSufficientValidators() public {
        _stakeAsPublisher(alice);
        _submitPackage(alice);
        _joinValidator(bob);
        _joinValidator(carol);

        // Build two approval sigs (meets 67% of 2 = 2).
        Registry.ValidatorSig[] memory sigs = new Registry.ValidatorSig[](2);
        sigs[0] = _makeValidatorSig(bobKey,   bob,   CANONICAL, CONTENT_HASH, true);
        sigs[1] = _makeValidatorSig(carolKey, carol, CANONICAL, CONTENT_HASH, true);

        registry.finalizePackage(CANONICAL, sigs);

        assertEq(uint(registry.getStatus(CANONICAL)), uint(Registry.PackageStatus.Verified));
    }

    function test_finalizationFailsWithInsufficientApprovals() public {
        _stakeAsPublisher(alice);
        _submitPackage(alice);
        _joinValidator(bob);
        _joinValidator(carol);

        // Only 1 approval — not enough (quorum is 2).
        Registry.ValidatorSig[] memory sigs = new Registry.ValidatorSig[](1);
        sigs[0] = _makeValidatorSig(bobKey, bob, CANONICAL, CONTENT_HASH, true);

        vm.expectRevert();
        registry.finalizePackage(CANONICAL, sigs);
    }

    function test_invalidSignatureReverts() public {
        _stakeAsPublisher(alice);
        _submitPackage(alice);
        _joinValidator(bob);
        _joinValidator(carol);

        Registry.ValidatorSig[] memory sigs = new Registry.ValidatorSig[](2);
        sigs[0] = _makeValidatorSig(bobKey, bob, CANONICAL, CONTENT_HASH, true);
        // Carol's sig is garbled.
        sigs[1] = Registry.ValidatorSig({ validator: carol, signature: bytes("bad-sig"), approved: true });

        vm.expectRevert();
        registry.finalizePackage(CANONICAL, sigs);
    }

    // ── Revocation ────────────────────────────────────────────────────────────

    function test_publisherCanRevoke() public {
        _stakeAndVerify();

        vm.prank(alice);
        registry.revokePackage(CANONICAL, "Vulnerability found");

        assertEq(uint(registry.getStatus(CANONICAL)), uint(Registry.PackageStatus.Revoked));
    }

    function test_governanceCanRevokeAndSlash() public {
        _stakeAndVerify();
        uint256 stakeBefore = staking.stakedBalance(alice);

        // Governance calls through the multisig.
        vm.prank(address(governance));
        registry.revokePackage(CANONICAL, "Malicious code detected");

        assertEq(uint(registry.getStatus(CANONICAL)), uint(Registry.PackageStatus.Revoked));
        // Publisher should have been slashed.
        assertLt(staking.stakedBalance(alice), stakeBefore);
    }

    function test_revokedPackageCannotBeResubmitted() public {
        _stakeAndVerify();

        vm.prank(alice);
        registry.revokePackage(CANONICAL, "Compromised");

        vm.prank(alice);
        vm.expectRevert();
        registry.submitPackage(CANONICAL, CONTENT_HASH, IPFS_CID);
    }

    // ── Staking ───────────────────────────────────────────────────────────────

    function test_validatorUnbondingPeriod() public {
        _joinValidator(bob);
        assertTrue(staking.isActiveValidator(bob));

        vm.prank(bob);
        staking.initiateUnbonding();
        assertFalse(staking.isActiveValidator(bob));

        // Can't withdraw during unbonding period.
        vm.prank(bob);
        vm.expectRevert();
        staking.withdrawValidatorStake();

        // Fast-forward past unbonding period.
        vm.warp(block.timestamp + 8 days);
        vm.prank(bob);
        staking.withdrawValidatorStake(); // Should succeed now.
        assertEq(staking.stakedBalance(bob), 0);
    }

    function test_slashAfterThreeOffences() public {
        _joinValidator(bob);

        // Three slashes auto-eject the validator.
        vm.startPrank(address(registry));
        staking.slash(bob, 0.1 ether, "Offense 1");
        staking.slash(bob, 0.1 ether, "Offense 2");
        staking.slash(bob, 0.1 ether, "Offense 3");
        vm.stopPrank();

        assertFalse(staking.isActiveValidator(bob));
    }

    // ── Reputation ───────────────────────────────────────────────────────────

    function test_newValidatorStartsAt50() public {
        assertEq(reputation.scoreOf(bob), 50);
    }

    function test_approvalIncreasesReputation() public {
        vm.prank(address(registry));
        reputation.recordApproval(bob);
        assertGt(reputation.scoreOf(bob), 50);
    }

    // ── VRF ───────────────────────────────────────────────────────────────────

    function test_vrfSelectsDifferentSetsForDifferentPackages() public {
        address[] memory validators = new address[](10);
        for (uint i = 0; i < 10; i++) {
            validators[i] = makeAddr(string.concat("val", vm.toString(i)));
            _joinValidatorAddr(validators[i]);
        }

        // Select for two different packages.
        vm.roll(100);
        address[] memory setA = vrf.selectValidators("npm:express@4.0.0", validators);
        address[] memory setB = vrf.selectValidators("npm:lodash@4.0.0",  validators);

        // They should differ (very high probability with random seed).
        bool differs = false;
        for (uint i = 0; i < setA.length; i++) {
            if (setA[i] != setB[i]) { differs = true; break; }
        }
        assertTrue(differs);
    }

    // ── Governance ────────────────────────────────────────────────────────────

    function test_governanceProposalRequiresThreshold() public {
        // Propose changing the quorum.
        bytes memory callData = abi.encodeCall(registry.setQuorum, (75));

        vm.prank(alice);
        uint256 id = governance.submit(address(registry), callData, "Increase quorum to 75%");

        // Only Alice voted — threshold is 2.
        vm.prank(alice);
        governance.vote(id, true);

        // Not executed yet.
        (,,,Governance.ProposalStatus status,,) = governance.getProposal(id);
        assertEq(uint(status), uint(Governance.ProposalStatus.Pending));

        // Bob votes — threshold met, auto-executes.
        vm.prank(bob);
        governance.vote(id, true);

        (,,,status,,) = governance.getProposal(id);
        assertEq(uint(status), uint(Governance.ProposalStatus.Executed));
        assertEq(registry.quorumPct(), 75);
    }

    // ── Helpers ───────────────────────────────────────────────────────────────

    function _stakeAsPublisher(address who) internal {
        vm.prank(who);
        staking.stakeAsPublisher{value: 0.01 ether}();
    }

    function _submitPackage(address who) internal {
        vm.prank(who);
        registry.submitPackage(CANONICAL, CONTENT_HASH, IPFS_CID);
    }

    function _joinValidator(address who) internal {
        vm.prank(who);
        staking.joinAsValidator{value: 1 ether}();
    }

    function _joinValidatorAddr(address who) internal {
        vm.deal(who, 2 ether);
        vm.prank(who);
        staking.joinAsValidator{value: 1 ether}();
    }

    function _stakeAndVerify() internal {
        _stakeAsPublisher(alice);
        _submitPackage(alice);
        _joinValidator(bob);
        _joinValidator(carol);

        Registry.ValidatorSig[] memory sigs = new Registry.ValidatorSig[](2);
        sigs[0] = _makeValidatorSig(bobKey,   bob,   CANONICAL, CONTENT_HASH, true);
        sigs[1] = _makeValidatorSig(carolKey, carol, CANONICAL, CONTENT_HASH, true);
        registry.finalizePackage(CANONICAL, sigs);
    }

    /// Produce a real ECDSA signature from a validator private key.
    function _makeValidatorSig(
        uint256 privKey,
        address validator,
        string memory canonical,
        bytes32 contentHash,
        bool approved
    ) internal pure returns (Registry.ValidatorSig memory) {
        bytes32 digest = keccak256(
            abi.encodePacked(
                "\x19Ethereum Signed Message:\n32",
                keccak256(abi.encodePacked(canonical, contentHash))
            )
        );
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(privKey, digest);
        return Registry.ValidatorSig({
            validator: validator,
            signature: abi.encodePacked(r, s, v),
            approved:  approved
        });
    }
}
