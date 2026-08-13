// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

interface IProxiable {
    function proxiableUUID() external view returns (bytes32);
}

contract EmitTryCalls {
    event AdminChanged(address previousAdmin, address newAdmin);

    address internal admin;

    function _getAdmin() internal view returns (address) {
        return admin;
    }

    function _setAdmin(address newAdmin) internal {
        admin = newAdmin;
    }

    function _changeAdmin(address newAdmin) internal {
        emit AdminChanged(_getAdmin(), newAdmin);
        _setAdmin(newAdmin);
    }

    function _upgradeToAndCallUUPS(address newImplementation) internal {
        try IProxiable(newImplementation).proxiableUUID() returns (bytes32) {
            _setAdmin(newImplementation);
        } catch {
            revert("not UUPS");
        }
    }
}
