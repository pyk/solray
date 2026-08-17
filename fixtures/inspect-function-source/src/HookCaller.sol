// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import { HookContext } from "./HookContext.sol";

abstract contract HookCaller is HookContext {
    /// @notice Calls the virtual hook declared in another file.
    function run() external view returns (uint256) {
        return _hook();
    }
}
