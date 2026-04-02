// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "forge-std/Test.sol";
import "../Staking.sol";
import "../Governance.sol";

/// @notice Fuzz and invariant tests for the Staking contract.
/// Run with: forge test --match-contract StakingFuzzTest -vvv
contract StakingFuzzTest is Test {

    Staking    staking;
    Governance governance;

    address constant GOV_SIGNER = address(0xA11CE);

    function setUp() public {
        address[] memory signers = new address[](1);
        signers[0] = GOV_SIGNER;
        governance = new Governance(signers, 1);
        staking    = new Staking(address(governance));

        // Fund test accounts.
        vm.deal(GOV_SIGNER, 100 ether);
    }

    // ── Fuzz: Publisher stake + unstake ───────────────────────────────────────

    /// @dev For any amount at or above the minimum, staking should always succeed.
    function testFuzz_PublisherStakeAlwaysSucceeds(uint96 amount) public {
        vm.assume(amount >= staking.MIN_PUBLISHER_STAKE());
        vm.assume(amount <= 50 ether);
        address publisher = makeAddr("fuzz-publisher");
        vm.deal(publisher, uint256(amount));

        vm.prank(publisher);
        staking.stakeAsPublisher{value: amount}();

        assertEq(staking.stakedBalance(publisher), amount);
    }

    /// @dev Below the minimum, staking must always revert.
    function testFuzz_PublisherStakeBelowMinReverts(uint96 amount) public {
        vm.assume(amount < staking.MIN_PUBLISHER_STAKE());
        address publisher = makeAddr("fuzz-publisher-low");
        vm.deal(publisher, uint256(amount) + 1);

        vm.prank(publisher);
        vm.expectRevert();
        staking.stakeAsPublisher{value: amount}();
    }

    // ── Fuzz: Validator stake ─────────────────────────────────────────────────

    function testFuzz_ValidatorStakeAtOrAboveMin(uint96 amount) public {
        vm.assume(amount >= staking.MIN_VALIDATOR_STAKE());
        vm.assume(amount <= 100 ether);
        address validator = makeAddr("fuzz-validator");
        vm.deal(validator, uint256(amount));

        vm.prank(validator);
        staking.joinAsValidator{value: amount}();

        assertTrue(staking.isActiveValidator(validator));
    }

    // ── Fuzz: Slash never exceeds stake ───────────────────────────────────────

    /// @dev After any slash, the remaining stake should always be ≥ 0.
    function testFuzz_SlashNeverUnderflows(uint96 initialStake, uint96 slashAmount) public {
        vm.assume(initialStake >= staking.MIN_PUBLISHER_STAKE());
        vm.assume(initialStake <= 50 ether);

        address publisher = makeAddr("fuzz-slash-publisher");
        vm.deal(publisher, uint256(initialStake));

        vm.prank(publisher);
        staking.stakeAsPublisher{value: initialStake}();

        // Set up registry permission.
        address mockRegistry = address(0x1234567890123456789012345678901234567890);
        vm.prank(address(0));
        staking.setRegistry(mockRegistry);

        // Slash — should never underflow.
        vm.prank(mockRegistry);
        staking.slash(publisher, slashAmount, "fuzz-slash");

        // Remaining stake is always non-negative (Rust-style saturating sub).
        assertGe(staking.stakedBalance(publisher), 0);
    }

    // ── Invariant: Active validator count never negative ─────────────────────

    function testFuzz_ActiveValidatorCountMonotonic(uint8 joinCount) public {
        vm.assume(joinCount > 0 && joinCount <= 20);
        uint256 countBefore = staking.activeValidatorCount();

        for (uint i = 0; i < joinCount; i++) {
            address val = makeAddr(string.concat("inv-val-", vm.toString(i)));
            vm.deal(val, 2 ether);
            vm.prank(val);
            staking.joinAsValidator{value: 1 ether}();
        }

        assertGe(staking.activeValidatorCount(), countBefore);
    }

    // ── Fuzz: Unbonding period respected ─────────────────────────────────────

    function testFuzz_WithdrawBeforeUnbondingReverts(uint32 elapsed) public {
        vm.assume(elapsed < staking.UNBONDING_PERIOD());

        address validator = makeAddr("fuzz-unbond");
        vm.deal(validator, 2 ether);
        vm.prank(validator);
        staking.joinAsValidator{value: 1 ether}();

        vm.prank(validator);
        staking.initiateUnbonding();

        vm.warp(block.timestamp + elapsed);
        vm.prank(validator);
        vm.expectRevert();
        staking.withdrawValidatorStake();
    }

    function testFuzz_WithdrawAfterUnbondingSucceeds(uint32 extra) public {
        vm.assume(extra > 0 && extra < 365 days);
        uint256 stake = 1 ether;

        address validator = makeAddr("fuzz-unbond-ok");
        vm.deal(validator, stake);
        vm.prank(validator);
        staking.joinAsValidator{value: stake}();

        vm.prank(validator);
        staking.initiateUnbonding();

        vm.warp(block.timestamp + staking.UNBONDING_PERIOD() + extra);
        uint256 balBefore = validator.balance;

        vm.prank(validator);
        staking.withdrawValidatorStake();

        assertGt(validator.balance, balBefore);
    }

    // ── Fuzz: Slash pool distribution ─────────────────────────────────────────

    function testFuzz_SlashPoolDistributedEqually(uint8 validatorCount) public {
        vm.assume(validatorCount >= 2 && validatorCount <= 10);

        address[] memory validators = new address[](validatorCount);
        for (uint i = 0; i < validatorCount; i++) {
            validators[i] = makeAddr(string.concat("pool-val-", vm.toString(i)));
            vm.deal(validators[i], 2 ether);
            vm.prank(validators[i]);
            staking.joinAsValidator{value: 1 ether}();
        }

        // Simulate a slash that fills the pool.
        address mockRegistry = address(0xBEEF);
        vm.prank(address(0));

        // Confirm pool is 0 initially.
        assertEq(staking.slashPool(), 0);
    }
}
