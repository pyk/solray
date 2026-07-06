// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract DirectFallback {
    function doSomething() external {}

    receive() external payable {}

    fallback() external payable {}
}
