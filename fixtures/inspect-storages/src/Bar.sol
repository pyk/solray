// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract ContractA {
    uint256 public value;

    function update(uint256 newValue) external {
        value = newValue;
    }
}
