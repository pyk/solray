// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

interface IParent {
    function factory() external view returns (address);
    function addLiquidity(address tokenA, address tokenB, uint256 amountA) external returns (uint256);
}

interface IChildRouter is IParent {
    function removeLiquidity(uint256 liquidity) external returns (uint256);
}

contract Impl is IChildRouter {
    function factory() external pure override returns (address) {
        return address(0);
    }

    function addLiquidity(address tokenA, address tokenB, uint256 amountA) external pure override returns (uint256) {
        return 1;
    }

    function removeLiquidity(uint256 liquidity) external pure override returns (uint256) {
        return 1;
    }
}
