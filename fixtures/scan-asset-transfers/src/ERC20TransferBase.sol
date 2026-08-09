// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "./IERC20.sol";

contract ERC20TransferBase {
    IERC20 public baseToken;

    function _basePrivateTransfer(address to, uint256 amount) private {
        baseToken.transfer(to, amount);
    }
}
