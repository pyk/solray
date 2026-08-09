// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

contract Crlf {
    error Bad();

    uint256 public value;

    function run() external view returns (uint256) {
        if (value == 0) {
            revert Bad();
        }
        return value;
    }
}
