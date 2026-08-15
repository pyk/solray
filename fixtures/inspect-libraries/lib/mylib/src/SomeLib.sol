// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

/// @notice A utility library defined in a lib dependency.
library LibUtils {
    function toBytes32(address addr) internal pure returns (bytes32) {
        return bytes32(uint256(uint160(addr)));
    }
}
