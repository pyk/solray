// SPDX-License-Identifier: MIT
pragma solidity >=0.5.0 <0.8.0;

contract LegacyBase {
    constructor(uint256 amount) public {
        _mint(amount);
    }

    function _mint(uint256 amount) internal {
        amount;
    }
}

contract LegacyMid is LegacyBase {
    constructor(uint256 amount) public LegacyBase(amount) {}
}

contract LegacyChild is LegacyMid {
    constructor() public LegacyMid(0) {}
}
