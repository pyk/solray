// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

/// @notice An abstract contract defined in a library.
abstract contract LibAbstract {
    uint256 public baseValue;

    function computeBase() external view virtual returns (uint256);
}
