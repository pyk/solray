// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

library DataLib {
    struct Info {
        uint256 value;
        Status status;
    }

    enum Status {
        None,
        Active
    }
}
