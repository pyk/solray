// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "./Middle.sol";

contract Child is Middle {
    function foo() external override returns (uint256) {
        return 42;
    }
}
