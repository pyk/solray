// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

interface IFirst {
    function transfer(address to, uint256 value) external returns (bool);
}

interface ISecond {
    function transfer(address to, uint256 value) external returns (bool);
}

contract Unrelated {
    function run() external pure returns (uint256) {
        return 1;
    }
}
