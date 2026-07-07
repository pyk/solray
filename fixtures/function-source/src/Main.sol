// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

/// @title Main contract
contract Main {
    struct Data {
        uint256 value;
        address sender;
    }

    Data private _data;

    /// @notice Execute with a given value.
    /// @param x The value to store.
    function execute(uint256 x) public {
        _data = Data(x, msg.sender);
        _processData();
    }

    /// @notice Process the stored data.
    /// @return The computed result.
    function _processData() internal view returns (uint256) {
        return _compute(_data.value);
    }

    /// @notice Compute something from a value.
    /// @param val The input value.
    /// @return val + 1.
    function _compute(uint256 val) internal pure returns (uint256) {
        return val + 1;
    }
}
