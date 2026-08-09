// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract InheritedOverrideBase {
    function _beforeFallback() internal virtual {}
}

contract InheritedOverrideIntermediate is InheritedOverrideBase {
    function _beforeFallback() internal override virtual {
        super._beforeFallback();
    }
}

contract InheritedOverrideChild is InheritedOverrideIntermediate {
    function childFunction() external {}
}
