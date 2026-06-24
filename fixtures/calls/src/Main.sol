// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "./Helper.sol";

contract Base {
    uint256 public baseValue;

    function baseWork() internal {
        baseValue = 1;
    }
}

contract Main is Base {
    uint256 public data;
    Helper public helper;

    function execute(uint256 x) public {
        data = x;
        helper.assist(x);
        internalWork();
    }

    function internalWork() internal {
        data += 1;
        baseWork();
    }

    function readOnly() public view returns (uint256) {
        return data + baseValue;
    }
}
