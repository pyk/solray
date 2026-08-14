// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

/// @notice A pure math library.
library MathLib {
    function add(uint256 a, uint256 b) internal pure returns (uint256) {
        return a + b;
    }

    function sub(uint256 a, uint256 b) internal pure returns (uint256) {
        return a - b;
    }
}
