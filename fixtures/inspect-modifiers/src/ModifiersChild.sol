// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {ModifiersMiddle} from "./ModifiersMiddle.sol";

contract ModifiersChild is ModifiersMiddle {
    modifier onlyChild() {
        _;
    }
}
