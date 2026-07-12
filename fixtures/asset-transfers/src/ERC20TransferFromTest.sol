// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "./ERC20TransferFromBase.sol";
import "./SafeERC20.sol";

contract ERC20TransferFromTest is ERC20TransferFromBase {
    using SafeERC20 for IERC20;

    IERC20 public token;

    // Case 1: token.transferFrom inside a private function.
    function _privateTransferFrom(address from, address to, uint256 amount) private returns (bool) {
        return token.transferFrom(from, to, amount);
    }

    // Case 2: IERC20(token).transferFrom pattern (interface cast).
    function erc20CastTransferFrom(address from, address to, uint256 amount) external returns (bool) {
        return IERC20(address(token)).transferFrom(from, to, amount);
    }

    // Case 3: token.safeTransferFrom using the library via `using`.
    function erc20SafeTransferFromLib(address from, address to, uint256 amount) external {
        token.safeTransferFrom(from, to, amount);
    }

    // Case 4: SafeERC20.safeTransferFrom using the library directly.
    function erc20DirectSafeTransferFrom(address from, address to, uint256 amount) external {
        SafeERC20.safeTransferFrom(
            token,
            from,
            to,
            amount
        );
    }

    // Public function that calls the private function.
    function callPrivateTransferFrom(address from, address to, uint256 amount) external returns (bool) {
        return _privateTransferFrom(from, to, amount);
    }
}
