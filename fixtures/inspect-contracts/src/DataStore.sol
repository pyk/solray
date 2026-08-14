// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {MathLib} from "./MathLib.sol";

/// @notice A data store that uses a library.
contract DataStore {
    using MathLib for uint256;

    mapping(address => uint256) public balances;

    function deposit() external payable {
        balances[msg.sender] = balances[msg.sender].add(msg.value);
    }

    function withdraw(uint256 amount) external {
        balances[msg.sender] = balances[msg.sender].sub(amount);
    }
}
