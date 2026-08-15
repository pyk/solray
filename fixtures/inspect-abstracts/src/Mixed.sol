// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {LibAbstract} from "mylib/SomeLib.sol";

/// @notice An abstract contract defined alongside a concrete contract and a library.
abstract contract AbstractBase {
    function getImplementation() external virtual returns (address);
}

/// @notice A concrete contract that inherits from AbstractBase and LibAbstract.
contract Concrete is AbstractBase, LibAbstract {
    function getImplementation() external override returns (address) {
        return address(this);
    }

    function computeBase() external view override returns (uint256) {
        return baseValue;
    }
}

/// @notice A utility library defined in the same file.
library Utils {
    function add(uint256 a, uint256 b) internal pure returns (uint256) {
        return a + b;
    }
}
