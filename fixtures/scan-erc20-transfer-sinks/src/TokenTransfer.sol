// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

interface IERC20 {
    function safeTransfer(address to, uint256 amount) external;
    function transfer(address to, uint256 amount) external returns (bool);
}

contract TokenSender {
    IERC20 public token;

    function sendToken(address to, uint256 amount) external {
        token.safeTransfer(to, amount);
    }

    function sendTokenDirect(address to, uint256 amount) external returns (bool) {
        return token.transfer(to, amount);
    }

    function sendMultiple(address to, uint256 amount0, uint256 amount1) external {
        token.safeTransfer(to, amount0);
        token.safeTransfer(to, amount1);
    }
}

library TokenLib {
    function send(IERC20 token_, address to, uint256 amount) internal {
        token_.safeTransfer(to, amount);
    }

    function transferDirect(IERC20 token_, address to, uint256 amount) internal returns (bool) {
        return token_.transfer(to, amount);
    }
}
