// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract Crlf {
    // leading comment

    function run() external view returns (uint256) {
        return helper();
    }

    function helper() internal pure returns (uint256) {
        return 1;
    }
}
