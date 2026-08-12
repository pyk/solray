// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

interface IParentA {}

interface IParentB {}

interface IChildContract is IParentA, IParentB {
    function childValue() external view returns (uint256);
}

abstract contract AbstractBase {
    function baseValue() public pure virtual returns (uint256) {
        return 1;
    }
}

contract ConcreteBase is AbstractBase {
    function concreteValue() public pure returns (uint256) {
        return 2;
    }
}

contract ContainerRef {
    constructor() {
        abi.encodeCall(ConcreteBase.concreteValue, ());
        abi.encodeCall(IChildContract.childValue, ());
    }
}
