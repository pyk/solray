// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "./CrossFileFnBase.sol";

contract CrossFileFnUser is CrossFileFnBase {
    /// @notice Process a value using the base helper and the constant.
    function process(uint256 value) external pure returns (uint256) {
        uint256 core = _doCoreWork(value);
        return core + PREFIX;
    }
}
