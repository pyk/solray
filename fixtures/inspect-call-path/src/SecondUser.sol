// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "./Library.sol";

contract SecondUser {
    using Lib for uint256;

    function secondUserWork() external returns (uint256) {
        uint256 x = 0;
        return x.libWork();
    }
}
