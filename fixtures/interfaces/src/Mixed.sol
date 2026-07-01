// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {IDependency} from "mylib/SomeLib.sol";

/// @notice An interface defined alongside a concrete contract.
interface IArrayUtils {
    function indexOf(uint256[] calldata items, uint256 value) external pure returns (uint256);
}

/// @notice A concrete contract that implements IDependency.
contract MyContract is IDependency {
    function ping() external pure override returns (bytes4) {
        return this.ping.selector;
    }
}
