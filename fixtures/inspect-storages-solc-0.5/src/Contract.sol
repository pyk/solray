// SPDX-License-Identifier: GPL-3.0
pragma solidity =0.5.12;

contract Store {
    mapping(address => uint256) public balances;
    address public owner;
    uint256 public total;
    bool public active;
}

contract Empty {}

contract Base {
    uint256 public value;
}

contract Child is Base {
    function set(uint256 v) external {
        value = v;
    }
}
