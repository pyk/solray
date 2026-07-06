// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract ContractA {
    function entrypointOne(string memory value) external {}

    function payMe() external payable {}

    function readOnly() external pure returns (uint256) {
        return 1;
    }
}
