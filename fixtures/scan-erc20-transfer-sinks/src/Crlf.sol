// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

interface IERC20 {
    function safeTransfer(address to, uint256 amount) external;
}

contract CrlfSender {
    IERC20 public token;

    function sendFirst(address to, uint256 amount) external {
        token.safeTransfer(to, amount);
    }

    function sendSecond(address to, uint256 amount) external {
        if (amount > 0) {
            token.safeTransfer(to, amount);
        }
    }
}
