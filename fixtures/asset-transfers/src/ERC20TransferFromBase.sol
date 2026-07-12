// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "./IERC20.sol";

contract ERC20TransferFromBase {
    IERC20 public baseToken;

    function _basePrivateTransferFrom(address from, address to, uint256 amount) private {
        baseToken.transferFrom(from, to, amount);
    }
}
