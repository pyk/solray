// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

abstract contract HookContext {
    /// @notice Virtual hook overridden by the most-derived contract.
    function _hook() internal view virtual returns (uint256) {
        return 0;
    }
}
