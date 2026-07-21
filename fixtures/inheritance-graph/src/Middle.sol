// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "./Base.sol";

abstract contract Middle is Base {
    function foo() external virtual returns (uint256);
}
