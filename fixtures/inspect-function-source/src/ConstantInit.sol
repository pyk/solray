// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

/// @notice Path-style offsets whose later constants are defined from earlier ones.
library ConstantInit {
    /// @dev The length of the bytes encoded address
    uint256 private constant ADDR_SIZE = 20;
    /// @dev The length of the bytes encoded fee
    uint256 private constant FEE_SIZE = 3;

    /// @dev The offset of a single token address and pool fee
    uint256 private constant NEXT_OFFSET = ADDR_SIZE + FEE_SIZE;
    /// @dev The offset of an encoded pool key
    uint256 private constant POP_OFFSET = NEXT_OFFSET + ADDR_SIZE;
    /// @dev The minimum length of an encoding that contains 2 or more pools
    uint256 private constant MULTIPLE_POOLS_MIN_LENGTH = POP_OFFSET + NEXT_OFFSET;

    /// @notice Returns true iff the path contains two or more pools
    function hasMultiplePools(bytes memory path) internal pure returns (bool) {
        return path.length >= MULTIPLE_POOLS_MIN_LENGTH;
    }
}

/// @notice State variable whose initializer names another constant.
contract CachedAmount {
    /// @dev Placeholder used before a real amount is written
    uint256 private constant DEFAULT_AMOUNT = type(uint256).max;

    /// @dev Transient cache of the last amount
    uint256 private amountCached = DEFAULT_AMOUNT;

    /// @notice Overwrite the cached amount
    function cache(uint256 value) external {
        amountCached = value;
    }
}
