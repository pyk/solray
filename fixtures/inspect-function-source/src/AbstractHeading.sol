// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

abstract contract AbstractBase {
    function baseValue() public pure virtual returns (uint256) {
        return 1;
    }
}

contract AbstractHeading {
    AbstractBase public base;

    function run() external view returns (uint256) {
        return base.baseValue();
    }
}
