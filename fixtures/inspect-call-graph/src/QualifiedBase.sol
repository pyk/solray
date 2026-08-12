// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract BaseToken {
    mapping(address => mapping(address => uint256)) internal _allowances;

    function _approve(address owner, address spender, uint256 amount) internal virtual {
        _allowances[owner][spender] = amount;
    }
}

contract QualifiedChild is BaseToken {
    function approve(address spender, uint256 amount) external returns (bool) {
        _approve(msg.sender, spender, amount);
        return true;
    }

    function _approve(address owner, address spender, uint256 amount) internal override {
        BaseToken._approve(owner, spender, amount);
    }
}
