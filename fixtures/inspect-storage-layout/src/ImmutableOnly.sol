// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract ImmutableOnly {
    address public immutable owner;
    uint256 public constant SCALE = 100;

    constructor(address _owner) {
        owner = _owner;
    }
}
