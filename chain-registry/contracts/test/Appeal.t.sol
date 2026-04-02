// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "forge-std/Test.sol";
import "../Appeal.sol";
import "../Registry.sol";
import "../Staking.sol";
import "../Reputation.sol";
import "../VRF.sol";
import "../Governance.sol";

contract AppealTest is Test {

    Appeal        appeal;
    ChainRegistry registry;
    Staking       staking;
    Reputation    reputation;
    VRF        vrf;
    Governance governance;

    address publisher = makeAddr("publisher");
    address panelist1 = makeAddr("panelist1");
    address panelist2 = makeAddr("panelist2");
    address panelist3 = makeAddr("panelist3");
    address govSigner = makeAddr("govSigner");

    string constant CANONICAL = "npm:bad-package@1.0.0";

    function setUp() public {
        address[] memory signers = new address[](1);
        signers[0] = govSigner;
        governance = new Governance(signers, 1);

        staking    = new Staking(address(governance));
        reputation = new Reputation(address(governance));
        vrf        = new VRF(address(governance));
        registry   = new ChainRegistry(
            address(staking), 
            address(reputation), 
            address(vrf), 
            address(governance),
            address(0)
        );
        staking.setRegistry(address(registry));
        reputation.setRegistry(address(registry));

        appeal = new Appeal(address(registry), address(staking), address(reputation), address(governance));

        // Add panelists.
        vm.startPrank(address(governance));
        appeal.addPanelist(panelist1);
        appeal.addPanelist(panelist2);
        appeal.addPanelist(panelist3);
        vm.stopPrank();

        vm.deal(publisher, 10 ether);
        vm.deal(panelist1, 1 ether);
    }

    function test_SubmitAppealRequiresBond() public {
        vm.prank(publisher);
        vm.expectRevert();
        appeal.appeal{value: 0.001 ether}(CANONICAL, "I'm innocent");
    }

    function test_SubmitAppealSucceeds() public {
        vm.prank(publisher);
        uint256 id = appeal.appeal{value: 0.1 ether}(CANONICAL, "No malicious code");

        (string memory canonical,,, Appeal.AppealStatus status,,,) = appeal.getAppeal(id);
        assertEq(canonical, CANONICAL);
        assertEq(uint(status), uint(Appeal.AppealStatus.Pending));
    }

    function test_PanelApprovalReturnsBond() public {
        vm.prank(publisher);
        uint256 id = appeal.appeal{value: 0.1 ether}(CANONICAL, "False positive");
        uint256 balBefore = publisher.balance;

        // Three panelists approve.
        vm.prank(panelist1); appeal.vote(id, true);
        vm.prank(panelist2); appeal.vote(id, true);
        vm.prank(panelist3); appeal.vote(id, true);

        (,,, Appeal.AppealStatus status,,,) = appeal.getAppeal(id);
        assertEq(uint(status), uint(Appeal.AppealStatus.Approved));
        // Bond returned to publisher.
        assertGt(publisher.balance, balBefore);
    }

    function test_PanelRejectionSlashesBond() public {
        // Publisher must stake first to be slashable.
        vm.prank(publisher);
        staking.stakeAsPublisher{value: 1 ether}();

        vm.prank(publisher);
        uint256 id = appeal.appeal{value: 0.1 ether}(CANONICAL, "Trying my luck");
        uint256 balBefore = publisher.balance;

        vm.prank(panelist1); appeal.vote(id, false);
        vm.prank(panelist2); appeal.vote(id, false);
        vm.prank(panelist3); appeal.vote(id, false);

        (,,, Appeal.AppealStatus status,,,) = appeal.getAppeal(id);
        assertEq(uint(status), uint(Appeal.AppealStatus.Rejected));
    }

    function test_CannotVoteTwice() public {
        vm.prank(publisher);
        uint256 id = appeal.appeal{value: 0.1 ether}(CANONICAL, "Please reconsider");

        vm.prank(panelist1);
        appeal.vote(id, true);

        vm.prank(panelist1);
        vm.expectRevert();
        appeal.vote(id, true);
    }

    function test_NonPanelistCannotVote() public {
        vm.prank(publisher);
        uint256 id = appeal.appeal{value: 0.1 ether}(CANONICAL, "Help");

        vm.prank(makeAddr("random"));
        vm.expectRevert();
        appeal.vote(id, true);
    }

    function test_AppealExpiresAfterWindow() public {
        vm.prank(publisher);
        uint256 id = appeal.appeal{value: 0.1 ether}(CANONICAL, "Waiting...");

        // Fast-forward past the 7-day window.
        vm.warp(block.timestamp + 8 days);
        appeal.expireAppeal(id);

        (,,, Appeal.AppealStatus status,,,) = appeal.getAppeal(id);
        assertEq(uint(status), uint(Appeal.AppealStatus.Expired));
    }

    function test_CannotExpireBeforeWindow() public {
        vm.prank(publisher);
        uint256 id = appeal.appeal{value: 0.1 ether}(CANONICAL, "Fresh appeal");

        vm.expectRevert();
        appeal.expireAppeal(id);
    }

    function test_OnlyGovernanceCanAddPanelist() public {
        vm.prank(makeAddr("rando"));
        vm.expectRevert();
        appeal.addPanelist(makeAddr("new-panelist"));
    }

    function testFuzz_AppealBondAlwaysAboveMin(uint96 bond) public {
        vm.assume(bond >= appeal.MIN_APPEAL_BOND());
        vm.assume(bond <= 5 ether);
        vm.deal(publisher, uint256(bond) + 0.01 ether);

        vm.prank(publisher);
        uint256 id = appeal.appeal{value: bond}(CANONICAL, "Fuzz test");
        (,, uint256 storedBond,,,, ) = appeal.getAppeal(id);
        assertEq(storedBond, bond);
    }
}
