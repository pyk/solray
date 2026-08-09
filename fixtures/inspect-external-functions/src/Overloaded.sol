// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract Overloaded {
    function doWork(uint256 value) external {}
    function doWork(uint256 value, bytes calldata data) external {}
    function doWork(uint256 value, bytes calldata data, address recipient) external {}
    function skip(uint256 value) external {}
    function skip(address recipient) external {}
}
