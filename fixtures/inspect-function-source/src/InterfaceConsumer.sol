// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "./IIndexTarget.sol";
import "./ITypeConversion.sol";

contract InterfaceConsumer {
    IIndexTarget public TARGET;

    /// @notice Use a target via member access on a state variable.
    function useTarget(uint256 x) external view returns (uint256) {
        return TARGET.doSomething(x);
    }

    /// @notice Use a type conversion to cast to an interface.
    function useConversion(address target, uint256 x) external view returns (uint256) {
        return ITypeConversion(target).doOther(x);
    }
}
