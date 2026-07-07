// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract IndexAccessTest {
    struct Item {
        uint256 value;
    }

    mapping(uint256 => Item) public items;

    function getItem(uint256 id) public view returns (uint256) {
        return items[id].value;
    }
}
