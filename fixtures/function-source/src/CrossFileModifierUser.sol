// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "./CrossFileModifierBase.sol";

contract CrossFileModifierUser is CrossFileModifierBase {
    uint256 private _value;

    /// @notice Sets a new value, requiring the caller to be qualified.
    function setValue(uint256 newValue) external onlyQualified {
        _value = newValue;
    }
}
