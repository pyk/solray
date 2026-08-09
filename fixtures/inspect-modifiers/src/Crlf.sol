// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract Crlf {
    uint256 public value;

    modifier onlyWhenActive() {
        _;
    }

    function setValue(uint256 newValue) external onlyWhenActive {
        value = newValue;
    }
}
