// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import { HookCaller } from "./HookCaller.sol";

contract VirtualCrossFile is HookCaller {
    /// @notice Most-derived override of the virtual hook.
    function _hook() internal view override returns (uint256) {
        return _helper();
    }

    /// @notice Helper pulled in by the override.
    function _helper() internal pure returns (uint256) {
        return 42;
    }
}
