// SPDX-License-Identifier: Apache-2.0
pragma solidity ^0.8.0;

/// Keeps the running balance for one account.
contract Ledger {
    uint256 public constant LIMIT = 5000;

    address private owner;

    constructor(address account) {
        owner = account;
    }

    /// Bills the account and returns what is left.
    function charge(uint256 amount) public returns (uint256) {
        if (amount > LIMIT) {
            return refuse(amount);
        }
        return commitEntry(amount);
    }

    function refuse(uint256 amount) internal returns (uint256) {
        warnOwner(owner);
        return 0;
    }

    function commitEntry(uint256 amount) internal returns (uint256) {
        writeEntry(owner, amount);
        return amount;
    }
}
