// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

library LibUtil {
    function isContract(address account) internal view returns (bool) {
        return account.code.length > 0;
    }
}
