// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract Helper {
    uint256 public counter;

    function assist(uint256 amount) public {
        counter = amount;
    }
}
