// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract ValueCaller {
    function callWithValue(address payable target) external payable {
        (bool success, ) = target.call{value: msg.value}("");
        require(success, "call failed");
    }
}
