// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {IModifiersInterface} from "./IModifiersInterface.sol";

contract ModifiersInterfaceChild is IModifiersInterface {
    modifier onlyChild() {
        _;
    }

    function foo() external pure {}
}
