// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

/// @title IInheritdoc Interface
/// @notice This interface defines doSomething and compute operations.
interface IInheritdoc {
    /// @notice Perform an action with a value.
    /// @param x The input value for the action.
    /// @return The result of the action.
    function doSomething(uint256 x) external returns (uint256);

    /// @notice Compute a result from two inputs.
    /// @param a The first input.
    /// @param b The second input.
    /// @return The computed sum.
    /// @dev This is a simple addition.
    function compute(uint256 a, uint256 b) external view returns (uint256);
}
