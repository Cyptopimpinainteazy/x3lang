// SPDX-License-Identifier: MIT
pragma solidity 0.8.24;

/**
 * @title LegacyPreimageHTLC  (TEST FIXTURE ONLY - never deploy)
 * @notice Verbatim logic of `NativeBnbHTLC` at commit 13054df, before 03a33a9
 *         added depositor-only release. Kept under test/ solely so the issue
 *         #42 red tests can show the leaked-preimage attack succeeding against
 *         the original preimage-only BSC withdrawal condition. Only the
 *         contract name and this header differ from the historical source.
 *
 *         Original header follows.
 * @notice Native BNB hash-time-locked escrow leg for experimental X3 swaps.
 * @dev No admin withdrawal or upgrade keys. Not audited. Not approved for mainnet use.
 *      A separate NEAR-side escrow and independent finality observer are required.
 *      SHA-256 operates on exactly 32 raw preimage bytes on BOTH chains.
 */
contract LegacyPreimageHTLC {
    enum Status {
        Absent,
        Locked,
        Claimed,
        Refunded
    }

    struct Swap {
        address initiator;
        address recipient;
        bytes32 hashlock;
        uint256 amount;
        uint64 expiresAt;
        Status status;
    }

    uint64 public constant MIN_LOCK_SECONDS = 300;
    uint64 public constant MAX_LOCK_SECONDS = 7 days;
    mapping(bytes32 => Swap) public swaps;
    uint256 public lockedNativeBalance;
    bool private entered;

    event Locked(
        bytes32 indexed swapId,
        address indexed initiator,
        address indexed recipient,
        bytes32 hashlock,
        uint256 amount,
        uint64 expiresAt
    );
    event Claimed(bytes32 indexed swapId, bytes32 secret, address indexed recipient, uint256 amount);
    event Refunded(bytes32 indexed swapId, address indexed initiator, uint256 amount);

    modifier nonReentrant() {
        require(!entered, "reentrant");
        entered = true;
        _;
        entered = false;
    }

    function lock(bytes32 swapId, address recipient, bytes32 hashlock, uint64 expiresAt) external payable nonReentrant {
        require(swapId != bytes32(0), "zero swap id");
        require(swaps[swapId].status == Status.Absent, "swap id exists");
        require(recipient != address(0) && hashlock != bytes32(0), "invalid recipient/hashlock");
        require(msg.value > 0, "zero escrow");
        require(expiresAt > block.timestamp + MIN_LOCK_SECONDS, "deadline too soon");
        require(expiresAt <= block.timestamp + MAX_LOCK_SECONDS, "deadline too far");
        swaps[swapId] = Swap(msg.sender, recipient, hashlock, msg.value, expiresAt, Status.Locked);
        lockedNativeBalance += msg.value;
        emit Locked(swapId, msg.sender, recipient, hashlock, msg.value, expiresAt);
    }

    function claim(bytes32 swapId, bytes32 secret) external nonReentrant {
        Swap storage s = swaps[swapId];
        require(s.status == Status.Locked, "not locked");
        require(msg.sender == s.recipient, "not recipient");
        require(block.timestamp < s.expiresAt, "expired");
        require(sha256(abi.encodePacked(secret)) == s.hashlock, "wrong secret");
        s.status = Status.Claimed;
        lockedNativeBalance -= s.amount;
        (bool success,) = payable(s.recipient).call{value: s.amount}("");
        require(success, "claim transfer failed");
        emit Claimed(swapId, secret, s.recipient, s.amount);
    }

    function refund(bytes32 swapId) external nonReentrant {
        Swap storage s = swaps[swapId];
        require(s.status == Status.Locked, "not locked");
        require(msg.sender == s.initiator, "not initiator");
        require(block.timestamp >= s.expiresAt, "not expired");
        s.status = Status.Refunded;
        lockedNativeBalance -= s.amount;
        (bool success,) = payable(s.initiator).call{value: s.amount}("");
        require(success, "refund transfer failed");
        emit Refunded(swapId, s.initiator, s.amount);
    }

    receive() external payable {
        revert("use lock");
    }
}
