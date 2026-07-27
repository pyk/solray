// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "./IParentA.sol";
import "./IParentB.sol";

interface IChild is IParentA, IParentB {
    function doChild(uint256 x) external view returns (uint256);
}
