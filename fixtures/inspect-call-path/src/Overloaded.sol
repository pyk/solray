// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract Overloaded {
    function run() external {
        _work(1);
    }

    function _work(uint256 value) internal {}

    function _work(uint256 a, uint256 b) internal {}
}
