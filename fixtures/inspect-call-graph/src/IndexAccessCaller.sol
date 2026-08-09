// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract Helper {
    function doWork() public pure returns (bool) {
        return true;
    }
}

contract IndexAccessCaller {
    mapping(address => Helper) private helpers;

    function run(address sender) public {
        _approve(_msgSender(), sender, helpers[_msgSender()].doWork());
    }

    function _approve(address owner, address sender, bool ok) internal {}

    function _msgSender() internal view returns (address) {
        return msg.sender;
    }
}
