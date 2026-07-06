// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract ContractA {
    function barWrite() external {}

    function barRead() external view returns (uint256) {
        return 1;
    }
}
