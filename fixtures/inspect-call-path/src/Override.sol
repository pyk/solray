// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

interface IOverride {
    function transfer(address to, uint256 value) external returns (bool);
}

contract Override is IOverride {
    function transfer(address to, uint256 value) public returns (bool) {
        return true;
    }
}
