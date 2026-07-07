// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

/// @title ModifierRef contract
contract ModifierRef {
    uint256 private _count;

    modifier onlyOwner() {
        require(msg.sender == address(this), "not owner");
        _;
    }

    modifier nonZero(uint256 value) {
        require(value > 0, "zero");
        _;
    }

    /// @notice Increment by a value.
    /// @param value The amount to increment.
    function increment(uint256 value) external onlyOwner nonZero(value) {
        _count += value;
    }
}
