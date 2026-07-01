// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

/// @notice An interface defined in its own file.
interface IPrimary {
    function owner() external view returns (address);
}
