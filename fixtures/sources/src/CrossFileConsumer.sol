// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "./TypesLib.sol";

contract CrossFileConsumer {
    function translate(TypesLib.Point memory p) public pure returns (TypesLib.Point memory) {
        TypesLib.Point memory q = p;
        return q;
    }
}
