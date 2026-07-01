// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

/// @notice An interface defined in a lib dependency.
interface IDependency {
    function ping() external pure returns (bytes4);
}
