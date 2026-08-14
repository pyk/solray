// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract NativeTransfer {
    function transferEth(address payable recipient, uint256 amount) external {
        recipient.transfer(amount);
    }

    function sendEth(address payable recipient, uint256 amount) external returns (bool) {
        return recipient.send(amount);
    }
}
