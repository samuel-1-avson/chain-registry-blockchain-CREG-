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
        require(msg.sender == owner, "Not owner");
        _mint(to, amount);
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
}

contract DeployScript is Script {
    function run() external {
        uint256 deployerPrivateKey = vm.envUint("PRIVATE_KEY");
        vm.startBroadcast(deployerPrivateKey);
        
        SimpleTokenDeploy token = new SimpleTokenDeploy();
        console.log("Token deployed to:", address(token));
        
        vm.stopBroadcast();
    }
}
