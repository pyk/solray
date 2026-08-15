// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

/// @notice A standalone library defined in its own file.
library SafeMath {
    function add(uint256 a, uint256 b) internal pure returns (uint256) {
        uint256 c = a + b;
        require(c >= a, "overflow");
        return c;
    }
}
