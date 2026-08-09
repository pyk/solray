// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract Base {
    uint256 public data;

    function inheritedWork() internal {
        data = 1;
    }
}
