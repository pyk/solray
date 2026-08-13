// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "./BaseInit.sol";

contract ChildInit is BaseInit {
    function initialize(address owner, string memory name, uint8 decimals) external {
        super.initialize(name, decimals);
    }
}
