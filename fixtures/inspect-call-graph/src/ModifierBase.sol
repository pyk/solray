// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract GuardBase {
    address private _owner;

    modifier onlyOwner() {
        _checkOwner();
        _;
    }

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
}

contract GuardUser is GuardBase {
    constructor(address initialOwner) GuardBase(initialOwner) {}

    function changeOwner(address newOwner) external onlyOwner {
        _transferOwnership(newOwner);
    }
}
