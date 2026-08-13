// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

interface IERC20 {
    function transfer(address to, uint256 amount) external returns (bool);
}

library TransferHelper {
    function safeTransfer(address token, address to, uint256 value) internal {
        (bool success, bytes memory data) = token.call(abi.encodeWithSelector(IERC20.transfer.selector, to, value));
        require(success && (data.length == 0 || abi.decode(data, (bool))));
    }
}

contract SingleStatementIf {
    address public token;

    function swap(address to, uint256 amount) external {
        if (amount > 0) TransferHelper.safeTransfer(token, to, amount);
    }

    function collect(address to, uint256 amount) external {
        if (amount > 0) {
            TransferHelper.safeTransfer(token, to, amount);
        }
    }
}
