// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

/// @title ModifierRef contract
contract ModifierRef {
    uint256 private _count;

    modifier onlyOwner() {
        _checkOwner();
        _;
    }

    modifier nonZero(uint256 value) {
        require(value > 0, "zero");
        _;
    }

    /// @notice Ensure the sender is the owner.
    function _checkOwner() internal view {
        require(msg.sender == address(this), "not owner");
    }

    /// @notice Increment by a value.
    /// @param value The amount to increment.
    function increment(uint256 value) external onlyOwner nonZero(value) {
        _count += value;
    }
}
