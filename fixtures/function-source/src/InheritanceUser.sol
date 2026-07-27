// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "./IChild.sol";

contract InheritanceUser {
    IChild public CHILD;

    /// @notice Use child interface via member access.
    function useChild(uint256 x) external view returns (uint256) {
        return CHILD.doChild(x);
    }
}
