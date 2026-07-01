// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

/// @notice An abstract contract defined in its own file.
abstract contract MyAbstract {
    uint256 public value;

    function compute() external virtual returns (uint256);
}
