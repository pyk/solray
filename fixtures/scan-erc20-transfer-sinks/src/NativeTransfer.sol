// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract NativeTransfer {
    function pay(address payable recipient, uint256 amount) external {
        recipient.transfer(amount);
    }
}
