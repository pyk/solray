// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

interface IToken {
    function transfer(address to, uint256 amount) external returns (bool);
    function balanceOf(address owner) external view returns (uint256);
}

contract TokenImpl is IToken {
    mapping(address => uint256) internal _balances;

    function transfer(address to, uint256 amount) public virtual override returns (bool) {
        _transfer(to, amount);
        return true;
    }

    function _transfer(address to, uint256 amount) internal {
        _balances[to] += amount;
    }

    function balanceOf(address owner) public view virtual override returns (uint256) {
        return _balances[owner];
    }
}

contract InterfaceFirst is IToken, TokenImpl {}
