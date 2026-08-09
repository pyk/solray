// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract TypeRefs {
    struct Point {
        uint256 x;
        uint256 y;
    }

    /// @notice Uses Point type in a local variable.
    function usePoint(Point memory p) public pure returns (uint256) {
        return p.x + p.y;
    }

    function createPoint(uint256 x, uint256 y) public pure returns (Point memory) {
        return Point(x, y);
    }

    function passThrough(Point memory p) public pure returns (Point memory) {
        Point memory q = p;
        return q;
    }
}
