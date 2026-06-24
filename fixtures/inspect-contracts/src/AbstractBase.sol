// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

/// @notice An abstract base contract that cannot be deployed directly.
abstract contract AbstractBase {
    address public owner;

    function foo() external virtual returns (uint256);
}
