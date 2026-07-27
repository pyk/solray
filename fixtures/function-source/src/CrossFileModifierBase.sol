// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

/// @notice Error emitted when an unqualified caller attempts an action.
error UnauthorizedCall();

contract CrossFileModifierBase {
    /// @notice Ensures the caller passes the qualification check.
    modifier onlyQualified() {
        if (!_checkQualified()) {
            revert UnauthorizedCall();
        }
        _;
    }

    /// @notice Returns true if the caller is qualified.
    function _checkQualified() internal pure returns (bool) {
        return true;
    }
}
