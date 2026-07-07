// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract BlockComment {
    /*
     *  A value holder with a regular block comment.
     *  This is not a NatSpec doc comment (no double-star).
     */
    struct Item {
        uint256 value;
    }

    function getItem() public pure returns (Item memory) {
        return Item(42);
    }
}
