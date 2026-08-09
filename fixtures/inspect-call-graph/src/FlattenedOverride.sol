// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

interface IFlattened {
    function process(uint256 value) external returns (bool);
    function total() external view returns (uint256);
    function balanceOf(address owner) external view returns (uint256);
}

contract ZetaBase is IFlattened {
    uint256 public total;
    mapping(address => uint256) public balanceOf;

    function process(uint256 value) external returns (bool) {
        return _process(value);
    }

    function _process(uint256 value) internal pure returns (bool) {
        return value > 0;
    }
}

contract Child is ZetaBase {}
