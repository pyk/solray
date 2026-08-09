// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract CrossFileFnBase {
    /// @notice Internal helper to do the core work.
    function _doCoreWork(uint256 x) internal pure returns (uint256) {
        return x * 2;
    }

    /// @notice Constant prefix used in computation.
    uint256 internal constant PREFIX = 42;
}
