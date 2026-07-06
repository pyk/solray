// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract ContractB {
    function update(address target) external {}

    function charge() external payable {}

    function count() external view returns (uint256) {
        return 2;
    }
}
