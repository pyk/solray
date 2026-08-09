// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract NatspecBlock {
    /**
     * @notice This function uses block-comment natspec.
     * @param x The input value.
     * @return The computed result.
     */
    function compute(uint256 x) public pure returns (uint256) {
        return x + 1;
    }
}
