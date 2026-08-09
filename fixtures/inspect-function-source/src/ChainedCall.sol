// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

/// @notice A library providing chainable numeric operations.
library ChainLib {
    /// @notice Step one: add one.
    function step1(uint256 x) internal pure returns (uint256) {
        return x + 1;
    }

    /// @notice Step two: double.
    function step2(uint256 x) internal pure returns (uint256) {
        return x * 2;
    }

    /// @notice Step three: add three.
    function step3(uint256 x) internal pure returns (uint256) {
        return x + 3;
    }
}

/// @notice Contract that uses chained library calls via `using`.
contract ChainedCall {
    using ChainLib for uint256;

    /// @notice Chain multiple steps on a value.
    /// @param start The initial value.
    /// @return The final computed value.
    function run(uint256 start) external pure returns (uint256) {
        return start.step1().step2().step3();
    }
}
