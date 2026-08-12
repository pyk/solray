// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

/// @notice A library with a used and an unused sibling function.
library LibraryScopeLib {
    error UsedError();
    error UnusedError();

    /// @notice Adds one, or reverts with UsedError.
    function used(uint256 x) internal pure returns (uint256) {
        if (x == 0) revert UsedError();
        return x + 1;
    }

    /// @notice Doubles, or reverts with UnusedError.
    function unused(uint256 x) internal pure returns (uint256) {
        if (x == 0) revert UnusedError();
        return x * 2;
    }
}

/// @notice Consumes the library through a qualified call.
contract LibraryScopeUser {
    /// @notice Calls the used library function.
    /// @param x The input value.
    /// @return The computed result.
    function run(uint256 x) external pure returns (uint256) {
        return LibraryScopeLib.used(x);
    }
}
