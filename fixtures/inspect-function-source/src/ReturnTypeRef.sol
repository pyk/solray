// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract ReturnTypeRef {
    struct Widget {
        uint256 id;
        string name;
    }

    /// @notice Returns a Widget. The Widget type only appears in the
    ///         return type, not in the body.
    function makeWidget(uint256 id, string memory name)
        public
        pure
        returns (Widget memory w)
    {
        assembly {
            mstore(w, 42)
        }
    }
}
