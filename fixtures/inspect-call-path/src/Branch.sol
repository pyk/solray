// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract Branch {
    function isContract(address account) internal pure returns (bool) {
        return account != address(0);
    }

    function viaA() internal pure {
        isContract(address(1));
    }

    function viaB() internal pure {
        isContract(address(2));
    }

    function entry() external pure {
        viaA();
        viaB();
    }
}
