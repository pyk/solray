// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

interface IERC20 {
    function safeTransfer(address to, uint256 amount) external;
    function transfer(address to, uint256 amount) external returns (bool);
}

contract TestTokenSender {
    IERC20 public token;

    function testSend(address to, uint256 amount) external {
        token.safeTransfer(to, amount);
    }
}
