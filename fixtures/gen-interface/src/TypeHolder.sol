// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {DataLib} from "./DataLib.sol";
import {OtherLib} from "./OtherLib.sol";

contract TypeHolder {
    function info() external pure returns (DataLib.Info memory) {
        DataLib.Info memory value;
        return value;
    }

    function status() external pure returns (DataLib.Status) {
        return DataLib.Status.None;
    }

    function other() external pure returns (OtherLib.Info memory) {
        OtherLib.Info memory value;
        return value;
    }

    function otherStatus() external pure returns (OtherLib.Status) {
        return OtherLib.Status.Off;
    }
}
