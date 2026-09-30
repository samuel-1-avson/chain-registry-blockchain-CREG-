// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import "forge-std/Script.sol";
import "../Governance.sol";
import "../Staking.sol";
import "../Reputation.sol";
import "../VRF.sol";
import "../Registry.sol";
import "../Appeal.sol";
import "../ZKVerifier.sol";
import "../CregToken.sol";
import "../ValidatorRewards.sol";
import "../PinningRewards.sol";

/// @notice Deploys the full chain-registry contract suite to Sepolia.
/// @dev Optional VRF and ZK components remain unset until real coordinator
///      configuration and a real verifying key are supplied. No placeholder
///      verifier or coordinator is deployed by this script.
///      Run with:
///        forge script contracts/script/DeploySepolia.s.sol:DeploySepolia \
///          --rpc-url $SEPOLIA_RPC_URL --broadcast --verify -vvvv
contract DeploySepolia is Script {

    uint256 internal constant GENESIS_VALIDATOR_STAKE = 1000 ether;

    Governance    public governance;
    Staking       public staking;
    Reputation    public reputation;
    VRF           public vrf;
    ChainRegistry public registry;
    Appeal        public appeal;
    ZKVerifier    public zkVerifier;
    CregToken     public cregToken;
    ValidatorRewards public validatorRewards;
    PinningRewards public pinningRewards;
    uint256 public stakingDeployBlock;

    function run() external {
        uint256 deployerKey = vm.envUint("DEPLOYER_KEY");
        address deployer    = vm.addr(deployerKey);
        uint256 threshold   = vm.envOr("GOVERNANCE_THRESHOLD", uint256(2));
        address[] memory signers = _parseSigners(deployer, threshold);

        console.log("=== Chain Registry Sepolia Deployment ===");
        console.log("Deployer:  ", deployer);
        console.log("Signers:   ", signers.length);
        console.log("Threshold: ", threshold);
        console.log("Chain ID:  ", block.chainid);

        require(block.chainid == 11155111, "This script is for Sepolia (chain ID 11155111)");

        vm.startBroadcast(deployerKey);

        governance = new Governance(signers, threshold);
        reputation = new Reputation(address(governance));
        // Registry stores the VRF address but does not call it in this alpha
        // deployment. Four validators all join the mandatory committee.
        vrf        = VRF(address(0));

        // CregToken: initial supply remains with the deployer until separate
        // allocations are approved and transferred under the tokenomics policy.
        cregToken = new CregToken(deployer, deployer, deployer, deployer);

        // Faucet funding is a separate policy and governance gate. The deploy
        // script never transfers the initial supply to a public hot wallet.

        staking    = new Staking(address(governance), address(cregToken));
        stakingDeployBlock = block.number;
        validatorRewards = new ValidatorRewards(
            address(staking),
            address(cregToken),
            address(governance),
            deployer
        );
        pinningRewards = new PinningRewards(address(cregToken));

        // No ZK verification key ceremony has been completed for this alpha.
        // Registry.zkValidationEnabled starts false and must remain false.
        zkVerifier = ZKVerifier(address(0));

        registry   = new ChainRegistry(
            address(staking),
            address(reputation),
            address(vrf),
            address(governance),
            address(zkVerifier)
        );

        appeal = new Appeal(address(registry), address(staking), address(reputation), address(governance));

        // Wire contracts together.
        staking.setContracts(address(registry), address(reputation));
        reputation.setRegistry(address(registry));
        cregToken.approve(address(validatorRewards), type(uint256).max);
        cregToken.transferOwnership(address(governance));

        vm.stopBroadcast();

        // Optionally seed the first validator from CREG_BRIDGE_KEY.
        _seedFirstValidator(deployerKey, deployer);
        _configureFinalizeRelay(deployerKey);

        console.log("Governance:", address(governance));
        console.log("Staking:   ", address(staking));
        console.log("Reputation:", address(reputation));
        console.log("VRF:       ", address(vrf));
        console.log("ZKVerifier:", address(zkVerifier));
        console.log("Registry:  ", address(registry));
        console.log("Appeal:    ", address(appeal));
        console.log("CregToken: ", address(cregToken));
        console.log("ValRewards:", address(validatorRewards));
        console.log("PinRewards:", address(pinningRewards));

        _writeManifest(deployer);
        console.log("=== Deployment complete ===");
    }

    function _parseSigners(address deployer, uint256 threshold) internal view returns (address[] memory) {
        try vm.envString("GENESIS_SIGNERS") returns (string memory raw) {
            if (bytes(raw).length > 0) {
                return vm.parseJsonAddressArray(raw, "$");
            }
        } catch {}

        // Default: deployer + bridge signer if available.
        try vm.envUint("CREG_BRIDGE_KEY") returns (uint256 bridgeKey) {
            address bridgeSigner = vm.addr(bridgeKey);
            if (bridgeSigner != deployer) {
                address[] memory s = new address[](2);
                s[0] = deployer;
                s[1] = bridgeSigner;
                return s;
            }
        } catch {}

        // If threshold > 1 but we only have deployer, warn but proceed.
        if (threshold > 1) {
            console.log("WARNING: threshold > 1 but only deployer signer available.");
        }
        address[] memory s = new address[](1);
        s[0] = deployer;
        return s;
    }

    function _seedFirstValidator(uint256 deployerKey, address deployer) internal {
        if (governance.threshold() != 1) {
            console.log("Seed skipped: multisig admission requires a separate approved proposal");
            return;
        }
        uint256 bridgeKey;
        try vm.envUint("CREG_BRIDGE_KEY") returns (uint256 k) {
            bridgeKey = k;
        } catch {
            console.log("Seed skipped: CREG_BRIDGE_KEY unset");
            return;
        }
        address bridgeSigner = vm.addr(bridgeKey);
        if (bridgeSigner == deployer) {
            console.log("Seed skipped: bridge signer equals deployer");
            return;
        }

        vm.startBroadcast(deployerKey);
        cregToken.transfer(bridgeSigner, GENESIS_VALIDATOR_STAKE);
        vm.stopBroadcast();

        vm.startBroadcast(bridgeKey);
        cregToken.approve(address(staking), GENESIS_VALIDATOR_STAKE);
        staking.applyToBeValidator(GENESIS_VALIDATOR_STAKE);
        vm.stopBroadcast();

        vm.startBroadcast(deployerKey);
        bytes memory cd = abi.encodeWithSelector(
            Staking.approveValidator.selector,
            bridgeSigner
        );
        uint256 proposalId = governance.submit(
            address(staking),
            cd,
            "genesis: seed first validator via governance"
        );
        governance.vote(proposalId, true);
        vm.stopBroadcast();

        console.log("Genesis validator seeded:", bridgeSigner);
    }

    /// @dev ISSUE-008: enable finalize relay allowlist on testnet so only the chain
    ///      node bridge (or configured relayer) may call finalizePackage.
    function _configureFinalizeRelay(uint256 deployerKey) internal {
        if (governance.threshold() != 1) {
            console.log("Finalize relay setup deferred: multisig requires co-signed proposal");
            return;
        }
        if (!vm.envOr("ENFORCE_FINALIZE_RELAYS", true)) {
            console.log("Finalize relay enforcement skipped (ENFORCE_FINALIZE_RELAYS=false)");
            return;
        }

        address relay = vm.envOr("FINALIZE_RELAY_ADDRESS", address(0));
        if (relay == address(0)) {
            try vm.envUint("CREG_BRIDGE_KEY") returns (uint256 bridgeKey) {
                relay = vm.addr(bridgeKey);
            } catch {
                console.log("Finalize relay skipped: set FINALIZE_RELAY_ADDRESS or CREG_BRIDGE_KEY");
                return;
            }
        }

        vm.startBroadcast(deployerKey);

        bytes memory authorizeRelay = abi.encodeWithSelector(
            ChainRegistry.setPackageFinalizeRelay.selector,
            relay,
            true
        );
        uint256 authId = governance.submit(
            address(registry),
            authorizeRelay,
            "testnet: authorize finalizePackage relay"
        );
        governance.vote(authId, true);

        bytes memory enforce = abi.encodeWithSelector(
            ChainRegistry.setEnforceFinalizeRelays.selector,
            true
        );
        uint256 enforceId = governance.submit(
            address(registry),
            enforce,
            "testnet: enforce finalizePackage relay allowlist"
        );
        governance.vote(enforceId, true);

        vm.stopBroadcast();

        console.log("Finalize relay authorized:", relay);
        console.log("enforceFinalizeRelays: true");
    }

    function _writeManifest(address deployer) internal {
        string memory outputPath = vm.envOr(
            "DEPLOYMENT_MANIFEST_PATH",
            string("contracts/deployments/sepolia-latest.json")
        );

        string memory p1 = string.concat(
            '{\n',
            '  "chainId":    "', vm.toString(block.chainid),       '",\n',
            '  "deployedAt": "', vm.toString(block.timestamp),     '",\n',
            '  "deployer":   "', vm.toString(deployer),           '",\n',
            '  "governance": "', vm.toString(address(governance)), '",\n',
            '  "staking":    "', vm.toString(address(staking)),    '",\n',
            '  "stakingDeployBlock": "', vm.toString(stakingDeployBlock), '",\n',
            '  "reputation": "', vm.toString(address(reputation)), '",\n'
        );
        string memory p2 = string.concat(
            '  "vrf":        "', vm.toString(address(vrf)),        '",\n',
            '  "zkVerifier": "', vm.toString(address(zkVerifier)), '",\n',
            '  "registry":   "', vm.toString(address(registry)),   '",\n',
            '  "appeal":     "', vm.toString(address(appeal)),     '",\n',
            '  "cregToken":  "', vm.toString(address(cregToken)),  '",\n',
            '  "validatorRewards": "', vm.toString(address(validatorRewards)), '",\n',
            '  "pinningRewards": "', vm.toString(address(pinningRewards)), '"\n',
            '}'
        );
        string memory m = string.concat(p1, p2);

        try vm.createDir("contracts/deployments", true) {} catch {}
        try vm.writeFile(outputPath, m) {
            console.log("Manifest:", outputPath);
        } catch {
            console.log("Manifest write skipped:", outputPath);
        }
    }
}
