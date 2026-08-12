// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

interface IChildToken {
    function balanceOf(address owner) external view returns (uint256);
}

contract ChildTokenImpl is IChildToken {
    mapping(address => uint256) internal _balances;

    function balanceOf(address owner) public view virtual override returns (uint256) {
        return _balances[owner];
    }
}

contract InterfaceChild is IChildToken, ChildTokenImpl {
    function checkBalance(address owner) external view returns (uint256) {
        return balanceOf(owner);
    }
}
