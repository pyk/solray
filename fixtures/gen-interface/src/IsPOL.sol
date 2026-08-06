// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

interface IsPOL {
    function balanceOf(address account) external view returns (uint256);
    function mint(address to) external;
    function totalStaked() external view returns (uint256);
}
