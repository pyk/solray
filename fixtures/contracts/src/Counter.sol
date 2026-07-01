// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

/// @notice A simple counter contract.
contract Counter {
    uint256 public count;

    function increment() external {
        count += 1;
    }

    function decrement() external {
        count -= 1;
    }

    function getCount() external view returns (uint256) {
        return count;
    }
}
