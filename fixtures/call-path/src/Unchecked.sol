// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract UncheckedTarget {
    uint256 public data;

    function internalWork() internal {
        data = 1;
    }

    function externalWork() external {
        unchecked {
            internalWork();
        }
    }
}
