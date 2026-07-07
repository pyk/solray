// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "./ViewsBase.sol";

contract ViewsChild is ViewsBase {
    function setValue(uint256 newValue) external {
        value = newValue;
    }
}
