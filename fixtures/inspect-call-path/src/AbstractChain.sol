// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

abstract contract ChainBase {
    function _implementation() internal view virtual returns (address);
}

contract ChainMid is ChainBase {
    function _implementation() internal view virtual override returns (address) {
        return address(this);
    }

    function run() external {
        _implementation();
    }
}

contract ChainTop is ChainMid {
    function trigger() external {
        _implementation();
    }
}
