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
import "../testnet/DevZKVerifier.sol";

/// @notice Deploys the full chain-registry contract suite.
/// @dev forge script contracts/script/Deploy.s.sol --rpc-url $RPC_URL --broadcast --verify -vvvv
contract DeployChainRegistry is Script {

    Governance    public governance;
    Staking       public staking;
    Reputation    public reputation;
    VRF           public vrf;
    ChainRegistry public registry;
    Appeal        public appeal;
    address       public zkVerifier;
    CregToken     public cregToken;

    function run() external {
        uint256 deployerKey = vm.envUint("DEPLOYER_KEY");
        address deployer    = vm.addr(deployerKey);
        address[] memory signers   = _parseSigners(deployer);
        uint256          threshold = vm.envOr("GOVERNANCE_THRESHOLD", uint256(1));

        console.log("=== Chain Registry Deployment ===");
        console.log("Deployer:  ", deployer);
        console.log("Signers:   ", signers.length);
        console.log("Threshold: ", threshold);

        vm.startBroadcast(deployerKey);

        governance = new Governance(signers, threshold);
        reputation = new Reputation(address(governance));
        vrf        = new VRF(address(1), bytes32(0), 0, address(governance));

        // CregToken must be deployed before Staking — Staking holds a reference to it.
        // All 42M max supply tokens go to deployer for local dev; adjust for production.
        cregToken = new CregToken(deployer, deployer, deployer, deployer);

        // Fund the faucet (Anvil account #1) with 20M tCREG so drip works immediately.
        address faucetAddr = vm.envOr("FAUCET_ADDRESS", address(0x70997970C51812dc3A010C7d01b50e0d17dc79C8));
        cregToken.transfer(faucetAddr, 20_000_000 ether);

        // Staking now requires the CregToken address for CREG-based staking.
        staking    = new Staking(address(governance), address(cregToken));

        // Local Anvil deployments use a permissive verifier so the bridge path
        // can exercise rollup settlement without a production Groth16 key set.
        zkVerifier = address(new DevZKVerifier());

        registry   = new ChainRegistry(
            address(staking),
            address(reputation),
            address(vrf),
            address(governance),
            zkVerifier
        );

        appeal = new Appeal(address(registry), address(staking), address(reputation), address(governance));

        // Wire contracts together — setContracts replaces the old setRegistry.
        staking.setContracts(address(registry), address(reputation));
        reputation.setRegistry(address(registry));

        vm.stopBroadcast();

        console.log("Governance:", address(governance));
        console.log("Staking:   ", address(staking));
        console.log("Reputation:", address(reputation));
        console.log("VRF:       ", address(vrf));
        console.log("ZKVerifier:", zkVerifier);
        console.log("Registry:  ", address(registry));
        console.log("Appeal:    ", address(appeal));
        console.log("CregToken: ", address(cregToken));

        _writeManifest(deployer);
        console.log("=== Deployment complete ===");
    }

    function _parseSigners(address deployer) internal view returns (address[] memory) {
        try vm.envString("GENESIS_SIGNERS") returns (string memory raw) {
            if (bytes(raw).length > 0) {
                return vm.parseJsonAddressArray(raw, "$");
            }
        } catch {}
        address[] memory s = new address[](1);
        s[0] = deployer;
        return s;
    }

    function _writeManifest(address deployer) internal {
        string memory canonicalPath = "contracts/deployments/latest.json";
        string memory testnetPath = "testnet/artifacts/testnet-contracts.json";
        string memory m = string.concat(
            '{\n',
            '  "deployer":   "', vm.toString(deployer),           '",\n',
            '  "governance": "', vm.toString(address(governance)), '",\n',
            '  "staking":    "', vm.toString(address(staking)),    '",\n',
            '  "reputation": "', vm.toString(address(reputation)), '",\n',
            '  "vrf":        "', vm.toString(address(vrf)),        '",\n',
            '  "zkVerifier": "', vm.toString(zkVerifier), '",\n',
            '  "registry":   "', vm.toString(address(registry)),   '",\n',
            '  "appeal":     "', vm.toString(address(appeal)),     '",\n',
            '  "cregToken":  "', vm.toString(address(cregToken)),  '",\n',
            '  "chainId":    "', vm.toString(block.chainid),       '",\n',
            '  "deployedAt": "', vm.toString(block.timestamp),     '"\n',
            '}'
        );

        try vm.envString("DEPLOYMENT_MANIFEST_PATH") returns (string memory configuredPath) {
            if (bytes(configuredPath).length > 0) {
                canonicalPath = configuredPath;
            }
        } catch {}

        try vm.createDir("contracts/deployments", true) {} catch {}
        try vm.createDir("testnet/artifacts", true) {} catch {}

        _tryWriteManifest(canonicalPath, m);
        if (keccak256(bytes(testnetPath)) != keccak256(bytes(canonicalPath))) {
            _tryWriteManifest(testnetPath, m);
        }
    }

    function _tryWriteManifest(string memory outputPath, string memory manifest) internal {
        try vm.writeFile(outputPath, manifest) {
            console.log("Manifest:", outputPath);
        } catch {
            console.log("Manifest write skipped:", outputPath);
        }
    }
}
