// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract FunctionTypeParam {
    function run(uint256 amount) external {
        _writeCheckpoint(_add, amount);
    }

    function _writeCheckpoint(
        function(uint256, uint256) pure returns (uint256) op,
        uint256 delta
    ) private returns (uint256) {
        return op(0, delta);
    }

    function _add(uint256 a, uint256 b) private pure returns (uint256) {
        return a + b;
    }
}
