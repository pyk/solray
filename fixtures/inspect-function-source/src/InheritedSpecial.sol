// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract InheritedSpecialParent {
    receive() external payable {}

    fallback() external payable {}
}

contract InheritedSpecialChild is InheritedSpecialParent {
    function childFunction() external {}
}
