// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {ModifiersBase} from "./ModifiersBase.sol";

contract ModifiersMiddle is ModifiersBase {
    modifier onlyMiddle() {
        _;
    }
}
