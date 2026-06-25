// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract ContractB {
    bool public active;

    function flip() external {
        active = !active;
    }
}
