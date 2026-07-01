// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {LibUtils} from "mylib/SomeLib.sol";

/// @notice A utility library defined alongside a contract.
library ArrayUtils {
    function indexOf(uint256[] storage arr, uint256 val) internal view returns (uint256) {
        for (uint256 i = 0; i < arr.length; i++) {
            if (arr[i] == val) return i;
        }
        revert("not found");
    }
}

/// @notice A concrete contract that uses ArrayUtils and LibUtils.
contract MyContract {
    using ArrayUtils for uint256[];
    using LibUtils for address;

    uint256[] private data;

    function push(uint256 val) external {
        data.push(val);
    }
}
