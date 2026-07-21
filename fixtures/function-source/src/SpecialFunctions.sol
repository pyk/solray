// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "./SpecialFunctionsImportedParent.sol";

contract SpecialFunctionsParent {
    function parentFunction() internal pure returns (uint256) {
        return 2;
    }
}

contract SpecialFunctions is SpecialFunctionsParent, SpecialFunctionsImportedParent {
    constructor() {
        sameContractFunction();
    }

    receive() external payable {
        parentFunction();
    }

    fallback() external payable {
        importedParentFunction();
    }

    function sameContractFunction() internal pure returns (uint256) {
        return 1;
    }
}
