// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "forge-std/Test.sol";
import "../Staking.sol";
import "../Reputation.sol";
import "../Governance.sol";
import "../CregToken.sol";
import "../Registry.sol";
import "../VRF.sol";
import "../ZKVerifier.sol";

/// @notice Regression tests for findings AUD-07 and AUD-08 in
///         docs/CREG-DEEP-DIVE-AUDIT-2026-11-14.md. Each test asserts the
///         intended behaviour.
contract StakingAuditFindingsTest is Test {
    Staking staking;
    Governance governance;
    CregToken cregToken;
    Reputation reputation;

    uint256 constant N_ACTIVE = 4;
    uint256[N_ACTIVE] activeKeys;
    address[N_ACTIVE] activeAddrs;

    uint256 constant STAKE_AMT = 200 ether;

    function setUp() public {
        address[] memory govSigners = new address[](1);
        govSigners[0] = address(this);
        governance = new Governance(govSigners, 1);

        cregToken = new CregToken(address(this), address(this), address(this), address(this));
        staking = new Staking(address(governance), address(cregToken));
        reputation = new Reputation(address(governance));
        staking.setContracts(address(0xBEEF), address(reputation));

        for (uint256 i = 0; i < N_ACTIVE; i++) {
            uint256 key = uint256(keccak256(abi.encodePacked("audit-active-", i)));
            activeKeys[i] = key;
            activeAddrs[i] = vm.addr(key);
            _fundAndApply(activeAddrs[i]);
            vm.prank(address(governance));
            staking.approveValidator(activeAddrs[i]);
        }
    }

    // ── Helpers ───────────────────────────────────────────────────────────────

    function _fundAndApply(address who) internal {
        cregToken.transfer(who, STAKE_AMT);
        _apply(who);
    }

    function _apply(address who) internal {
        vm.startPrank(who);
        cregToken.approve(address(staking), STAKE_AMT);
        staking.applyToBeValidator(STAKE_AMT);
        vm.stopPrank();
    }

    function _expire(address who) internal {
        vm.warp(block.timestamp + staking.APPLICATION_TIMEOUT() + 1);
        staking.expireApplication(who);
    }

    function _validatorStake(address who) internal view returns (uint256 stake) {
        (stake,,,,,) = staking.validators(who);
    }

    function _isActive(address who) internal view returns (bool) {
        (, Staking.ValidatorState state,,,,) = staking.validators(who);
        return state == Staking.ValidatorState.Active;
    }

    /// Signatures from the first `count` original actives over the current
    /// EIP-712 admission digest, sorted by signer address.
    function _bundle(uint256 count, address applicant, uint256 nonce)
        internal
        view
        returns (address[] memory signers, bytes[] memory sigs)
    {
        uint256[] memory keys = new uint256[](count);
        signers = new address[](count);
        for (uint256 i = 0; i < count; i++) {
            keys[i] = activeKeys[i];
            signers[i] = activeAddrs[i];
        }
        for (uint256 i = 1; i < count; i++) {
            uint256 j = i;
            while (j > 0 && signers[j - 1] > signers[j]) {
                (signers[j - 1], signers[j]) = (signers[j], signers[j - 1]);
                (keys[j - 1], keys[j]) = (keys[j], keys[j - 1]);
                j--;
            }
        }
        bytes32 digest = staking.consensusMessageHash(applicant, STAKE_AMT, nonce);
        sigs = new bytes[](count);
        for (uint256 i = 0; i < count; i++) {
            (uint8 v, bytes32 r, bytes32 s) = vm.sign(keys[i], digest);
            sigs[i] = abi.encodePacked(r, s, v);
        }
    }

    /// Put `amount` CREG into the slash pool by slashing a publisher.
    function _fillSlashPool(uint256 amount, string memory label) internal {
        address publisher = makeAddr(label);
        cregToken.transfer(publisher, amount);
        vm.startPrank(publisher);
        cregToken.approve(address(staking), amount);
        staking.stakeAsPublisher(amount);
        vm.stopPrank();
        vm.prank(address(governance));
        staking.slash(publisher, amount, "audit test");
    }

    // ── AUD-07: re-applying validators ────────────────────────────────────────

    function test_AUD07_reappliedValidatorIsCountedOnce() public {
        address returning = makeAddr("returning-validator");
        _fundAndApply(returning);
        _expire(returning);
        _apply(returning);
        vm.prank(address(governance));
        staking.approveValidator(returning);

        assertEq(
            staking.activeValidatorCount(),
            N_ACTIVE + 1,
            "a validator that re-applied must be counted once"
        );
    }

    function test_AUD07_reapplicationsDoNotRaiseAdmissionQuorum() public {
        address returning = makeAddr("returning-validator");
        _fundAndApply(returning);
        _expire(returning);
        _apply(returning);
        _expire(returning);
        _apply(returning);
        vm.prank(address(governance));
        staking.approveValidator(returning);

        // Five distinct actives; four signatures (80%) satisfy the >= 2/3 rule.
        address applicant = makeAddr("next-applicant");
        _fundAndApply(applicant);
        (address[] memory signers, bytes[] memory sigs) = _bundle(4, applicant, 1);
        staking.approveByConsensus(applicant, 1, signers, sigs);

        assertTrue(_isActive(applicant), "4 of 5 distinct actives must admit an applicant");
    }

    // ── AUD-08: slash-pool epochs ─────────────────────────────────────────────

    function test_AUD08_equalWeightsReceiveEqualShares() public {
        _fillSlashPool(100 ether, "slashed-publisher");
        vm.prank(address(governance));
        staking.commitSlashPoolEpoch();

        for (uint256 i = 0; i < N_ACTIVE; i++) {
            uint256 before = _validatorStake(activeAddrs[i]);
            vm.prank(activeAddrs[i]);
            staking.claimSlashPoolShare();
            assertEq(
                _validatorStake(activeAddrs[i]) - before,
                25 ether,
                "share must not depend on claim order"
            );
        }
    }

    function test_AUD08_nextEpochCanBeCommittedAfterAllClaims() public {
        _fillSlashPool(100 ether, "slashed-publisher");
        vm.prank(address(governance));
        staking.commitSlashPoolEpoch();
        for (uint256 i = 0; i < N_ACTIVE; i++) {
            vm.prank(activeAddrs[i]);
            staking.claimSlashPoolShare();
        }

        _fillSlashPool(10 ether, "second-slashed-publisher");
        vm.prank(address(governance));
        staking.commitSlashPoolEpoch();
        assertEq(staking.slashPoolEpoch(), 2, "a second epoch must be committable");
    }

    function test_AUD08_unclaimedRemainderCarriesIntoNextEpoch() public {
        _fillSlashPool(100 ether, "slashed-publisher");
        vm.prank(address(governance));
        staking.commitSlashPoolEpoch();
        // Only three of four validators claim before the next commit.
        for (uint256 i = 0; i < N_ACTIVE - 1; i++) {
            vm.prank(activeAddrs[i]);
            staking.claimSlashPoolShare();
        }
        assertEq(staking.slashPoolEpochAmount(), 25 ether, "one share left unclaimed");

        _fillSlashPool(10 ether, "second-slashed-publisher");
        vm.prank(address(governance));
        staking.commitSlashPoolEpoch();
        assertEq(staking.slashPoolEpochTotal(), 35 ether, "remainder carried into the new epoch");
        assertEq(staking.slashPool(), 0, "pool moved into the epoch");
    }
}

/// @notice Regression test for finding AUD-09: a publisher's self-revocation
///         must not release the release bond before the liability window.
contract RegistryAuditFindingsTest is Test {
    ChainRegistry registry;
    Staking staking;
    Reputation reputation;
    Governance governance;
    CregToken cregToken;

    address alice = makeAddr("alice");

    string constant CANONICAL = "npm:audit-self-revoke@1.0.0";
    bytes32 constant CONTENT_HASH = keccak256("audit-tarball");
    string constant IPFS_CID = "bafybeigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzdi";

    function setUp() public {
        address[] memory signers = new address[](1);
        signers[0] = address(this);
        governance = new Governance(signers, 1);
        cregToken = new CregToken(address(this), address(this), address(this), address(this));
        staking = new Staking(address(governance), address(cregToken));
        reputation = new Reputation(address(governance));
        VRF vrf = new VRF(address(1), bytes32(0), 0, address(governance));

        uint256[2] memory a1 = [uint256(0), 0];
        uint256[2] memory zeros = [uint256(0), 0];
        uint256[2][] memory ic = new uint256[2][](2);
        ic[0] = [uint256(0), 0];
        ic[1] = [uint256(0), 0];
        ZKVerifier zkVerifier = new ZKVerifier(a1, zeros, zeros, zeros, zeros, zeros, zeros, ic);

        registry = new ChainRegistry(
            address(staking), address(reputation), address(vrf), address(governance), address(zkVerifier)
        );
        staking.setContracts(address(registry), address(reputation));
        reputation.setRegistry(address(registry));
    }

    uint256 constant STAKE = 2 ether;

    function _publishThenSelfRevoke() internal {
        cregToken.transfer(alice, STAKE);
        vm.startPrank(alice);
        cregToken.approve(address(staking), STAKE);
        staking.stakeAsPublisher(STAKE);
        registry.submitPackage(CANONICAL, CONTENT_HASH, IPFS_CID);
        vm.stopPrank();
        assertEq(
            staking.availablePublisherStake(alice),
            STAKE - staking.minPublisherStake(),
            "bond reserved on submit"
        );
        vm.prank(alice);
        registry.revokePackage(CANONICAL, "self-revoked", Staking.Severity.Low);
    }

    function test_AUD09_selfRevocationKeepsTheBondReserved() public {
        _publishThenSelfRevoke();
        assertEq(
            staking.availablePublisherStake(alice),
            STAKE - staking.minPublisherStake(),
            "self-revocation must not release the release bond"
        );
        vm.expectRevert();
        vm.prank(alice);
        staking.unstakeAsPublisher(STAKE);
    }

    function test_AUD09_governanceCanStillSlashAfterSelfRevocation() public {
        _publishThenSelfRevoke();
        bytes32 key = keccak256(abi.encodePacked(CANONICAL));
        uint256 poolBefore = staking.slashPool();

        vm.prank(address(governance));
        staking.resolvePublisherLiability(key, true, "malicious release");

        assertEq(staking.slashPool() - poolBefore, staking.minPublisherStake(), "bond slashed");
        assertEq(staking.stakedBalance(alice), STAKE - staking.minPublisherStake());
    }

    function test_AUD09_bondIsReleasedAfterTheLiabilityWindow() public {
        _publishThenSelfRevoke();
        bytes32 key = keccak256(abi.encodePacked(CANONICAL));

        vm.warp(block.timestamp + staking.PUBLISHER_LIABILITY_PERIOD());
        staking.releaseMaturedPublisherLiability(key);

        vm.prank(alice);
        staking.unstakeAsPublisher(STAKE);
        assertEq(staking.stakedBalance(alice), 0, "full stake withdrawable after the window");
    }
}
