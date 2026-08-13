// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "./MidToken.sol";

contract GuardedToken is MidToken {
    modifier whenNotPaused() {
        _;
    }

    function transfer(address to, uint256 amount)
        public
        override
        whenNotPaused
        returns (bool)
    {
        return super.transfer(to, amount);
    }
}
