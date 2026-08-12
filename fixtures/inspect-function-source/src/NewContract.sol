// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract NewBase {
    function _init(address owner) internal {
        owner;
    }

    constructor(address owner) {
        _init(owner);
    }
}

/**
 * @dev Helper contract created by NewContract.
 */
contract NewHelper is NewBase {
    function helper() internal {}

    constructor(address owner) NewBase(owner) {
        helper();
    }
}

contract NewContract {
    constructor() {
        new NewHelper(msg.sender);
    }
}
