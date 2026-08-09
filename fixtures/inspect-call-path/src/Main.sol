// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "./Library.sol";

contract Grandparent {
    uint256 public gpData;

    function grandparentWork() internal {
        gpData = 1;
    }

    function grandparentExternal() external {
        grandparentWork();
    }
}

contract Parent is Grandparent {
    uint256 public pData;

    function parentWork() internal {
        pData = 1;
        grandparentWork();
    }

    function parentExternal() external {
        parentWork();
    }
}

contract Target is Parent {
    using Lib for uint256;

    uint256 public tData;

    function targetInternal() internal {
        tData = 42;
        parentWork();
        uint256 x = 0;
        x.libWork();
    }

    function targetExternal() external {
        targetInternal();
    }

    function otherExternal() external {
        parentWork();
    }

    function externalCallingLib() external returns (uint256) {
        uint256 x = 0;
        return x.libWork();
    }

    receive() external payable {
        targetInternal();
    }

    fallback() external {
        targetInternal();
    }
}
