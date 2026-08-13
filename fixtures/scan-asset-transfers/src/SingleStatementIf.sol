// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "./IERC20.sol";
import "./SafeERC20.sol";

contract SingleStatementIf {
    IERC20 public token;

    function swap(address to, uint256 amount) external {
        if (amount > 0) SafeERC20.safeTransfer(token, to, amount);
    }

    function collect(address to, uint256 amount) external {
        if (amount > 0) {
            SafeERC20.safeTransfer(token, to, amount);
        }
    }
}
