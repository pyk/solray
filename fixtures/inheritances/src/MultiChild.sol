// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "./MultiBase.sol";
import "./AnotherBase.sol";

contract MultiChild is MultiBase, AnotherBase {
    uint256 public z;
}
