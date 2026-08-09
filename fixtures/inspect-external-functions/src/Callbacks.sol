// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract CallbackReceiver {
    function doSomething() external {}

    function onERC721Received(
        address,
        address,
        uint256,
        bytes calldata
    ) external pure returns (bytes4) {
        return 0x150b7a02;
    }

    function onERC1155Received(
        address,
        address,
        uint256,
        uint256,
        bytes calldata
    ) external pure returns (bytes4) {
        return 0xf23a6e61;
    }

    function readOnly() external view returns (uint256) {
        return 1;
    }
}
