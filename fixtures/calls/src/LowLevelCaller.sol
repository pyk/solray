// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract LowLevelCaller {
    function callWithPayload(address target, bytes calldata data) external {
        (bool success, ) = target.call(data);
        require(success, "call failed");
    }
}
