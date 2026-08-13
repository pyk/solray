// SPDX-License-Identifier: MIT
pragma solidity >=0.5.0 <0.8.0;

library OracleLike {
    struct Observation {
        uint32 blockTimestamp;
        bool initialized;
    }

    function write(Observation[8] storage self, uint16 index) internal {
        transform(self[index]);
    }

    function transform(Observation memory last) private pure {
        last.initialized = true;
    }
}

contract LegacyPool {
    using OracleLike for OracleLike.Observation[8];

    OracleLike.Observation[8] public observations;

    struct ModifyPositionParams {
        address owner;
    }

    function mint() external {
        _modifyPosition(ModifyPositionParams({owner: msg.sender}));
        observations.write(0);
    }

    function _modifyPosition(ModifyPositionParams memory params) private {
        params.owner;
    }
}
