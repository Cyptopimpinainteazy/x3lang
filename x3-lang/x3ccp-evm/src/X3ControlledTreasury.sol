// SPDX-License-Identifier: MIT
pragma solidity 0.8.24;

import "./X3Controlled.sol";

/// @title X3ControlledTreasury — an ETH treasury whose spending is governed by an X3Lang policy
/// @notice Every external function that moves value or changes state is `x3Controlled`, so the
///         installed `control` policy decides who may call it, with which arguments, how often and
///         until when. With no policy installed every controlled call is refused (fail closed).
contract X3ControlledTreasury is X3Controlled {
    error TransferFailed();
    error Reentered();

    event Deposited(address indexed from, uint256 amount);
    event Withdrawn(address indexed caller, address indexed to, uint256 amount);
    event Swept(address indexed caller, address indexed to, uint256 amount);
    event MemoSet(address indexed caller, bytes32 memo, bool urgent);

    bytes32 public memo;
    bool public memoUrgent;
    uint256 private _lock = 1;

    constructor() X3Controlled(msg.sender) {}

    modifier nonReentrant() {
        if (_lock != 1) revert Reentered();
        _lock = 2;
        _;
        _lock = 1;
    }

    receive() external payable {
        emit Deposited(msg.sender, msg.value);
    }

    function withdraw(address to, uint256 amount) external x3Controlled nonReentrant {
        (bool ok,) = payable(to).call{value: amount}("");
        if (!ok) revert TransferFailed();
        emit Withdrawn(msg.sender, to, amount);
    }

    /// @notice Sends the whole balance. Only callable if the installed policy allows `sweep(address)`.
    function sweep(address to) external x3Controlled nonReentrant {
        uint256 amount = address(this).balance;
        (bool ok,) = payable(to).call{value: amount}("");
        if (!ok) revert TransferFailed();
        emit Swept(msg.sender, to, amount);
    }

    function setMemo(bytes32 memo_, bool urgent) external x3Controlled {
        memo = memo_;
        memoUrgent = urgent;
        emit MemoSet(msg.sender, memo_, urgent);
    }
}
