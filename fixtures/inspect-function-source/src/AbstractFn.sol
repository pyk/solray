// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

abstract contract AbstractFn {
    function _approve(
        address owner,
        address spender,
        uint256 value
    ) internal virtual;

    function _transfer(
        address from,
        address to,
        uint256 value
    ) internal virtual;
}

abstract contract AbstractFnChild is AbstractFn {}
