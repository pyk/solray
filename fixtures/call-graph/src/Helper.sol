// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

interface IHelper {
    function doWork() external;
}

contract Helper is IHelper {
    uint256 public counter;

    function assist(uint256 amount) public {
        counter = amount;
    }

    function doWork() external {
        counter = 42;
    }
}
