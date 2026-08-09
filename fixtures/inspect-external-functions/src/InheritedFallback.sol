// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract ParentWithFallback {
    function parentFunc() external {}

    receive() external payable {}

    fallback() external payable {}
}

contract ChildIsFallback is ParentWithFallback {
    function childFunc() external {}
}
