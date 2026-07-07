// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract Overloaded {
    function beforeTokenTransfer(address sender, address recipient, uint256 amount) public {}

    function beforeTokenTransfer(address sender, address recipient) public {}
}
