// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract OwnerLib {
    address private _owner;

    function owner() public view returns (address) {
        return _owner;
    }

    function _checkOwner() internal view {
        require(msg.sender == owner(), "unauthorized");
    }

    function _transferOwnership(address newOwner) internal {
        _owner = newOwner;
    }

    constructor(address initialOwner) {
        _transferOwnership(initialOwner);
    }

    function renounceOwnership() public {
        _checkOwner();
        _transferOwnership(address(0));
    }

    function transferOwnership(address newOwner) public {
        _checkOwner();
        _transferOwnership(newOwner);
    }
}
