// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract VirtualBase {
    function _beforeFallback() internal virtual {}

    function _implementation() internal virtual view returns (address) {
        return address(0);
    }

    fallback() external payable {
        _beforeFallback();
        _implementation();
    }
}

contract VirtualIntermediate is VirtualBase {
    function _beforeFallback() internal override virtual {
        super._beforeFallback();
    }

    function _implementation() internal override view returns (address) {
        return address(1);
    }
}

contract VirtualChild is VirtualIntermediate {
    function childFunction() external {}
}
