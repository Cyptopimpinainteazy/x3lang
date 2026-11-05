// SPDX-License-Identifier: MIT
pragma solidity 0.8.24;

/**
 * @title NativeBnbHTLC
 * @notice Owner-authorized native BNB escrow with hashlock commitment for bilateral X3 swaps.
 * @dev No admin withdrawal or upgrade keys. Not audited. Not approved for mainnet use.
 *      A separate NEAR-side escrow and independent finality observer are required.
 *      SHA-256 operates on exactly 32 raw preimage bytes on BOTH chains.
 *      IMPORTANT: Claim requires the ORIGINAL DEPOSITOR to authorize the BNB payout.
 *      A known preimage alone is not sufficient to claim BNB, because a failed
 *      NEAR claim can disclose the preimage without delivering NEAR credit.
 *      This is NOT a permissionless/trustless atomic swap: a dishonest depositor
 *      could receive NEAR then refuse to authorize BNB release to the LP.
 *      Only use with a trusted, controlled LP after independent finality checks.
 */
contract NativeBnbHTLC {
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
    event OwnerReleased(bytes32 indexed swapId, bytes32 secret, address indexed recipient, uint256 amount);
    event Refunded(bytes32 indexed swapId, address indexed initiator, uint256 amount);

    /// @dev Experimental contract is *not* deployable on BSC mainnet.
    constructor() {
        require(block.chainid == 97 || block.chainid == 31337, "testnet-only prototype");
    }

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

    /// @notice Source depositor MAY release BNB after independently verifying NEAR credit.
    /// @dev No automatic verification of NEAR state occurs on BSC. LP cannot call this.
    function ownerRelease(bytes32 swapId, bytes32 secret) external nonReentrant {
        Swap storage s = swaps[swapId];
        require(s.status == Status.Locked, "not locked");
        require(msg.sender == s.initiator, "depositor release required");
        require(block.timestamp < s.expiresAt, "expired");
        require(sha256(abi.encodePacked(secret)) == s.hashlock, "wrong secret");
        s.status = Status.Claimed;
        lockedNativeBalance -= s.amount;
        (bool success,) = payable(s.recipient).call{value: s.amount}("");
        require(success, "claim transfer failed");
        emit OwnerReleased(swapId, secret, s.recipient, s.amount);
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
