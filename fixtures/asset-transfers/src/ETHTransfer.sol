// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract ETHTransfer {
    // Case 4: receive function - flagged as ETH receiver.
    receive() external payable {}

    // Case 1: ETH send
    function ethSend(address payable to) external payable {
        bool sent = to.send(msg.value);
        require(sent);
    }

    // Case 2: ETH transfer
    function ethTransfer(address payable to) external payable {
        to.transfer(msg.value);
    }

    // Case 3: ETH call{value}
    function ethCall(address payable to) external payable {
        (bool ok, ) = to.call{value: msg.value}("");
        require(ok);
    }

    // Case 5: external payable function - flagged as ETH receiver.
    function acceptEth() external payable {}

    // Case 6: public payable function - also flagged as ETH receiver.
    function acceptEthPublic() public payable {}
}
