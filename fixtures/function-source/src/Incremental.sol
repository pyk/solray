// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract Incremental {
    struct Item {
        uint256 value;
    }

    function getValue(Item memory it) public pure returns (uint256) {
        return it.value;
    }
}
