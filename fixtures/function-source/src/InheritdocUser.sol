// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "./IInheritdoc.sol";

/// @title InheritdocUser contract
contract InheritdocUser is IInheritdoc {
    /// @inheritdoc IInheritdoc
    function doSomething(uint256 x) external pure returns (uint256) {
        return x + 1;
    }

    /// @inheritdoc IInheritdoc
    function compute(uint256 a, uint256 b) external pure returns (uint256) {
        return a + b;
    }
}
