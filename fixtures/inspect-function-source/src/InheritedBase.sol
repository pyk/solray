// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

abstract contract InheritedBase {
    error Bad();

    uint256 public value;

    function owner() public view virtual returns (uint256) {
        return value;
    }
}
