// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

import "./InheritedBase.sol";

contract Inherited is InheritedBase {
    function run() external view returns (uint256) {
        return owner();
    }
}
