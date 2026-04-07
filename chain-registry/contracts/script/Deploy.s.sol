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

/// @notice Deploys the full chain-registry contract suite.
/// @dev forge script contracts/script/Deploy.s.sol --rpc-url $RPC_URL --broadcast --verify -vvvv
contract DeployChainRegistry is Script {

    Governance    public governance;
    Staking       public staking;
    Reputation    public reputation;
    VRF           public vrf;
    ChainRegistry public registry;
    Appeal        public appeal;
    ZKVerifier    public zkVerifier;
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

        // Staking now requires the CregToken address for CREG-based staking.
        staking    = new Staking(address(governance), address(cregToken));

        // Phase 1: ZK Verifier setup (Dummy IC for dev)
        uint256[2] memory a1 = [uint256(0), 0];
        uint256[2] memory b2x = [uint256(0), 0];
        uint256[2] memory b2y = [uint256(0), 0];
        uint256[2] memory g2x = [uint256(0), 0];
        uint256[2] memory g2y = [uint256(0), 0];
        uint256[2] memory d2x = [uint256(0), 0];
        uint256[2] memory d2y = [uint256(0), 0];
        uint256[2][] memory ic = new uint256[2][](2);
        ic[0] = [uint256(0), 0];
        ic[1] = [uint256(0), 0];

        zkVerifier = new ZKVerifier(a1, b2x, b2y, g2x, g2y, d2x, d2y, ic);

        registry   = new ChainRegistry(
            address(staking),
            address(reputation),
            address(vrf),
            address(governance),
            address(zkVerifier)
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
        console.log("ZKVerifier:", address(zkVerifier));
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
            '  "zkVerifier": "', vm.toString(address(zkVerifier)), '",\n',
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
