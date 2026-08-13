// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

library HistoryLib {
    struct History {
        uint256 value;
    }

    function push(History storage self, uint256 value) internal {
        self.value = value;
    }
}

contract HookBase {
    function _mint(uint256 amount) internal virtual {
        _afterTransfer(amount);
    }

    function _afterTransfer(uint256 amount) internal virtual {}

    function transfer(uint256 amount) public {
        _afterTransfer(amount);
    }
}

contract HookToken is HookBase {
    using HistoryLib for HistoryLib.History;

    HistoryLib.History private _history;

    function mint(uint256 amount) external {
        _mint(amount);
    }

    function adjust(uint256 value) external {
        _history.push(value);
    }

    function _afterTransfer(uint256 amount) internal override {
        _history.push(amount);
    }
}
