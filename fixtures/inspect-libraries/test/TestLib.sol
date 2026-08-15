// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

/// @notice A library defined in the test directory.
library TestHelpers {
    function hash(string memory s) internal pure returns (bytes32) {
        return keccak256(bytes(s));
    }
}
