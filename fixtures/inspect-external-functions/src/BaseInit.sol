// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract BaseInit {
    bool private _initialized;

    modifier initializer() {
        require(!_initialized);
        _initialized = true;
        _;
    }

    function initialize(string memory name, uint8 decimals) public initializer {}
}
