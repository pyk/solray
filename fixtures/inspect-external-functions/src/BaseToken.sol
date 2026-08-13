// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract BaseToken {
    function name() public pure returns (string memory) {
        return "Base";
    }

    function symbol() public pure returns (string memory) {
        return "BASE";
    }

    function decimals() public pure returns (uint8) {
        return 18;
    }

    function transfer(address to, uint256 amount) public virtual returns (bool) {
        return true;
    }
}
