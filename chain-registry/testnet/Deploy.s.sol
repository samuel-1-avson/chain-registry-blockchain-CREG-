// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import "forge-std/Script.sol";

// Copy of SimpleToken for deployment
contract SimpleTokenDeploy {
    string public name = "Test CREG Token";
    string public symbol = "tCREG";
    uint8 public decimals = 18;
    uint256 public totalSupply;
    address public owner;
    address public faucet;
    mapping(address => uint256) public balanceOf;
    mapping(address => mapping(address => uint256)) public allowance;
    
    event Transfer(address indexed from, address indexed to, uint256 value);
    event Approval(address indexed owner, address indexed spender, uint256 value);
    
    constructor() {
        owner = msg.sender;
        _mint(msg.sender, 10_000_000 * 10**18);
    }
    
    function _mint(address to, uint256 amount) internal {
        totalSupply += amount;
        balanceOf[to] += amount;
        emit Transfer(address(0), to, amount);
    }
    
    function mint(address to, uint256 amount) external {
        require(msg.sender == owner || msg.sender == faucet, "Not authorized");
        _mint(to, amount);
    }

    /// @notice Public minting for testnet users (limited to 1000 CREG per call)
    function publicMint(uint256 amount) external {
        require(amount <= 1000 * 10**18, "Amount too high for testnet faucet");
        _mint(msg.sender, amount);
    }
    
    function setFaucet(address _faucet) external {
        require(msg.sender == owner, "Not owner");
        faucet = _faucet;
    }
    
    function transfer(address to, uint256 amount) public returns (bool) {
        require(balanceOf[msg.sender] >= amount, "Insufficient balance");
        balanceOf[msg.sender] -= amount;
        balanceOf[to] += amount;
        emit Transfer(msg.sender, to, amount);
        return true;
    }
    
    function approve(address spender, uint256 amount) public returns (bool) {
        allowance[msg.sender][spender] = amount;
        emit Approval(msg.sender, spender, amount);
        return true;
    }
    
    function transferFrom(address from, address to, uint256 amount) public returns (bool) {
        require(balanceOf[from] >= amount, "Insufficient balance");
        require(allowance[from][msg.sender] >= amount, "Insufficient allowance");
        allowance[from][msg.sender] -= amount;
        balanceOf[from] -= amount;
        balanceOf[to] += amount;
        emit Transfer(from, to, amount);
        return true;
    }
}

// Minimal testnet staking — inline to avoid import path issues in Foundry container
contract TestStakingDeploy {
    SimpleTokenDeploy public cregToken;
    address public operator;

    uint256 public minPublisherStake = 0.001 ether;
    uint256 public minValidatorStake = 0.1 ether;

    struct PublisherEntry { uint256 stake; bool isActive; }
    struct ValidatorEntry { uint256 stake; bool isActive; }

    mapping(address => PublisherEntry) public publishers;
    mapping(address => ValidatorEntry) public validators;

    event PublisherStaked(address indexed publisher, uint256 amount);
    event ValidatorStaked(address indexed validator, uint256 amount);

    constructor(address _cregToken) {
        cregToken = SimpleTokenDeploy(_cregToken);
        operator = msg.sender;
    }

    function stakeAsPublisher(uint256 amount) external {
        require(amount >= minPublisherStake, "Below minimum");
        cregToken.transferFrom(msg.sender, address(this), amount);
        publishers[msg.sender].stake += amount;
        publishers[msg.sender].isActive = true;
        emit PublisherStaked(msg.sender, amount);
    }

    function applyToBeValidator(uint256 amount) external {
        require(amount >= minValidatorStake, "Below minimum");
        cregToken.transferFrom(msg.sender, address(this), amount);
        validators[msg.sender].stake += amount;
        validators[msg.sender].isActive = true;
        emit ValidatorStaked(msg.sender, amount);
    }

    function getPublisherStake(address addr) external view returns (uint256) { return publishers[addr].stake; }
    function getValidatorStake(address addr) external view returns (uint256) { return validators[addr].stake; }
    function isPublisher(address addr) external view returns (bool) { return publishers[addr].isActive; }
    function isValidator(address addr) external view returns (bool) { return validators[addr].isActive; }
}

contract DeployScript is Script {
    function run() external {
        uint256 deployerPrivateKey = vm.envUint("DEPLOYER_KEY");
        address deployer = vm.addr(deployerPrivateKey);
        
        vm.startBroadcast(deployerPrivateKey);
        console.log("Deploying to Chain ID:", block.chainid);
        
        // Deploy token
        SimpleTokenDeploy token = new SimpleTokenDeploy();
        console.log("Token deployed to:", address(token));
        
        // Deploy staking with token reference
        TestStakingDeploy staking = new TestStakingDeploy(address(token));
        console.log("Staking deployed to:", address(staking));
        
        // Set up faucet address (Anvil account #1)
        address faucetAddr = vm.envOr("FAUCET_ADDRESS", address(0x70997970C51812dc3A010C7d01b50e0d17dc79C8));
        token.setFaucet(faucetAddr);
        console.log("Faucet set to:", faucetAddr);
        
        // Transfer initial tokens to faucet for distribution
        token.transfer(faucetAddr, 1_000_000 * 10**18);
        console.log("Transferred 1M tCREG to faucet");

        vm.stopBroadcast();
        
        // Write deployment manifest
        string memory manifest = string.concat(
            '{\n',
            '  "token": "', vm.toString(address(token)), '",\n',
            '  "staking": "', vm.toString(address(staking)), '",\n',
            '  "deployer": "', vm.toString(deployer), '",\n',
            '  "faucet": "', vm.toString(faucetAddr), '",\n',
            '  "chainId": "', vm.toString(block.chainid), '"\n',
            '}'
        );
        
        try vm.createDir("testnet/artifacts", true) {} catch {}
        try vm.writeFile("testnet/artifacts/testnet-contracts.json", manifest) {
            console.log("Manifest written to testnet/artifacts/testnet-contracts.json");
        } catch {
            console.log("Warning: Could not write manifest file");
        }
        
        console.log("=== Testnet deployment complete ===");
    }
}
