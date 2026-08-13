// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

library HistoryLib {
    struct Checkpoint {
        uint32 fromBlock;
        uint224 value;
    }

    struct History {
        Checkpoint[] _checkpoints;
    }

    function getAtBlock(History storage self, uint256) internal view returns (uint256) {
        if (self._checkpoints.length == 0) {
            return 0;
        }
        return self._checkpoints[0].value;
    }
}

contract HistoryUser {
    using HistoryLib for HistoryLib.History;

    HistoryLib.History private _history;

    function lookup(uint256 blockNumber) external view returns (uint256) {
        return _history.getAtBlock(blockNumber);
    }
}
