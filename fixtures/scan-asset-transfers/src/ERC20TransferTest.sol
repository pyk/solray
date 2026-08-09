// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "./ERC20TransferBase.sol";
import "./SafeERC20.sol";

contract ERC20TransferTest is ERC20TransferBase {
    using SafeERC20 for IERC20;

    IERC20 public token;

    // Case 1: token.transfer inside a private function.
    function _privateTransfer(address to, uint256 amount) private returns (bool) {
        return token.transfer(to, amount);
    }

    // Case 2: IERC20(token).transfer pattern (interface cast).
    function erc20CastTransfer(address to, uint256 amount) external returns (bool) {
        return IERC20(address(token)).transfer(to, amount);
    }

    // Case 3: token.safeTransfer using the library via `using`.
    function erc20SafeTransferLib(address to, uint256 amount) external {
        token.safeTransfer(to, amount);
    }

    // Case 4: SafeERC20.safeTransfer using the library directly.
    function erc20DirectSafeTransfer(address to, uint256 amount) external {
        SafeERC20.safeTransfer(token, to, amount);
    }

    // Public function that calls the private function.
    function callPrivateTransfer(address to, uint256 amount) external returns (bool) {
        return _privateTransfer(to, amount);
    }
}
