// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract Base {
    uint256 public baseValue;

    function baseWork() internal {
        baseValue = 1;
    }
}
