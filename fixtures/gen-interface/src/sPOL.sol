// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

contract sPOL {
    uint256 public totalStaked;

    function mint(address to) external {
        totalStaked += 1;
    }

    function balanceOf(address account) external view returns (uint256) {
        return totalStaked;
    }
}
