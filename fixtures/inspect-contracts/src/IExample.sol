// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

/// @notice An example interface.
interface IExample {
    function foo() external returns (uint256);
    function bar(address who) external view returns (bool);
}
