// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {ISharedBase} from "./ISharedBase.sol";

contract ModifierRoot is ISharedBase {
    modifier onlyRoot() {
        _;
    }
}
