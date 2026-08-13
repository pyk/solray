// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract BaseCtor {
    event Upgraded(address implementation);

    address internal impl;

    function _setImplementation(address newImplementation) internal {
        impl = newImplementation;
    }

    function _upgradeTo(address newImplementation) internal {
        _setImplementation(newImplementation);
        emit Upgraded(newImplementation);
    }

    constructor(address logic) {
        _upgradeTo(logic);
    }
}

contract ChildCtor is BaseCtor {
    address internal admin;

    function _setAdmin(address newAdmin) internal {
        admin = newAdmin;
    }

    constructor(address logic, address admin_) BaseCtor(logic) {
        _setAdmin(admin_);
    }
}
