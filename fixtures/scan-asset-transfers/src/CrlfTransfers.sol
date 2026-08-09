// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "./IERC20.sol";
import "./SafeERC20.sol";

contract CrlfTransfers {
    using SafeERC20 for IERC20;

    IERC20 public token;

    function send(address to, uint256 amount) external {
        token.transfer(to, amount);
        token.safeTransfer(to, amount);
        token.transferFrom(msg.sender, to, amount);
        token.safeTransferFrom(msg.sender, to, amount);
    }

    function acceptEth() external payable {}
}
