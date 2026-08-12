// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {OwnerLib} from "../shared/OwnerLib.sol";

contract User is OwnerLib {
    constructor() OwnerLib(msg.sender) {}

    function poke() public {
        _checkOwner();
    }
}
