// SPDX-License-Identifier: GPL-3.0
pragma solidity =0.5.12;

contract GemLike {
    function decimals() public view returns (uint256);
    function transfer(address, uint256) external returns (bool);
    function transferFrom(address, address, uint256) external returns (bool);
}

contract VatLike {
    function slip(bytes32, address, int256) external;
    function move(address, address, uint256) external;
}

contract GemJoin {
    mapping(address => uint256) public wards;
    VatLike public vat;
    bytes32 public ilk;
    GemLike public gem;
    uint256 public live;

    constructor(address vat_, bytes32 ilk_, address gem_) public {
        wards[msg.sender] = 1;
        live = 1;
        vat = VatLike(vat_);
        ilk = ilk_;
        gem = GemLike(gem_);
    }

    function cage() external {
        live = 0;
    }

    function join(address usr, uint256 wad) external {
        require(live == 1, "GemJoin/not-live");
        vat.slip(ilk, usr, int256(wad));
        require(gem.transferFrom(msg.sender, address(this), wad), "GemJoin/failed-transfer");
    }
}

contract Empty {}
