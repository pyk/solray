// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

library Lib {
    function libWork(uint256 self) internal pure returns (uint256) {
        return self + 1;
    }
}
