// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

struct Deposit {
    address from;
    uint256 amount;
}

enum Status {
    Inactive,
    Active
}

type Price is uint128;

interface IPriceOracle {
    function priceOf(address token) external view returns (uint256);
}

contract PoolBase {
    uint256 public baseValue;

    function inheritedView(uint256 x) external pure returns (uint256) {
        return x * 2;
    }
}

contract Pool is PoolBase {
    uint256 public totalSupply;
    mapping(address => uint256) public balances;
    address payable public feeRecipient;
    Price public listingPrice;
    Status public status;
    IPriceOracle public oracle;

    constructor() {
        totalSupply = 0;
    }

    function deposit(uint256 amount) external payable {
        totalSupply += amount;
    }

    function deposit(Deposit calldata deposit) external {
        balances[deposit.from] += deposit.amount;
    }

    function balanceOf(address account) external view returns (uint256) {
        return balances[account];
    }

    function transfer(address to, uint256 amount) external returns (bool) {
        balances[msg.sender] -= amount;
        balances[to] += amount;
        return true;
    }

    function currentStatus() external view returns (Status) {
        return status;
    }

    function applyStatuses(Status[] calldata statuses) external pure returns (Status[] memory) {
        return statuses;
    }

    function quote(Price price) external pure returns (Price) {
        return price;
    }

    function batch(Deposit[] calldata deposits) external returns (uint256) {
        return deposits.length;
    }

    function setOracle(IPriceOracle newOracle) external {
        oracle = newOracle;
    }

    function onERC721Received(
        address operator,
        address from,
        uint256 tokenId,
        bytes calldata data
    ) external returns (bytes4) {
        return this.onERC721Received.selector;
    }

    function internalOnly() internal pure returns (uint256) {
        return 1;
    }

    function privateOnly() private pure returns (uint256) {
        return 2;
    }

    receive() external payable {}

    fallback() external payable {}
}
