// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract ImplBase {
    function _impl() internal view virtual returns (address) {
        return address(this);
    }
}

contract DirectCall is ImplBase {
    modifier ifAdmin() {
        if (msg.sender == _admin()) {
            _;
        } else {
            _fallback();
        }
    }

    function _admin() internal view returns (address) {
        return address(this);
    }

    function _fallback() internal virtual {
        _impl();
    }

    function implementation() external ifAdmin returns (address impl) {
        impl = _impl();
    }
}
