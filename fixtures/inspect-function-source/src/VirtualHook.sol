// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract VirtualHookBase {
    function _mint(address to, uint256 amount) internal virtual {
        _afterTokenTransfer(address(0), to, amount);
    }

    function _afterTokenTransfer(address from, address to, uint256 amount) internal virtual {}
}

abstract contract VirtualHookVotes is VirtualHookBase {
    event VotesMoved(address from, address to, uint256 amount);

    function _mint(address to, uint256 amount) internal virtual override {
        super._mint(to, amount);
    }

    function _afterTokenTransfer(address from, address to, uint256 amount)
        internal
        virtual
        override
    {
        super._afterTokenTransfer(from, to, amount);
        _moveVotes(from, to, amount);
    }

    function _moveVotes(address from, address to, uint256 amount) private {
        emit VotesMoved(from, to, amount);
    }
}

contract VirtualHookToken is VirtualHookVotes {
    uint256 public totalDelegation;

    function mint(address to, uint256 amount) external {
        _mint(to, amount);
    }

    function _afterTokenTransfer(address from, address to, uint256 amount) internal override {
        super._afterTokenTransfer(from, to, amount);
        _updateDelegation(amount);
    }

    function _updateDelegation(uint256 amount) internal {
        totalDelegation += amount;
    }
}
