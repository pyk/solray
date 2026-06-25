// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract ContractA {
    uint256 public count;
    address public owner;

    function increment() external {
        count += 1;
    }
}
