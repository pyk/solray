// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "./Base.sol";

contract CrossFileTarget is Base {
    function entry() external {
        inheritedWork();
    }
}
