// SPDX-License-Identifier: MIT
pragma solidity 0.8.24;

/**
 * @notice Experimental TESTNET-ONLY BNB escrow requiring a 2-of-3 attestation
 * of FINALIZED NEAR escrow state before either side can receive BNB.
 * @dev This is a trusted-verifier settlement pilot, NOT a light client, atomic
 * bridge, fraud-proof verifier, or production-safe permissionless escrow.
 * A dishonest or censored 2-of-3 quorum can steal funds or lock them forever.
 * The off-chain observer/authentication system is NOT implemented here.
 */
contract AttestedBnbEscrow {
    enum State {
        Absent,
        Locked,
        Released,
        Refunded
    }
    enum NearOutcome {
        Claimed,
        Refunded,
        NotFunded
    }

    struct Swap {
        address depositor;
        address lp;
        bytes32 hashlock;
        bytes32 nearVaultHash;
        bytes32 nearRecipientHash;
        uint128 nearAmountYocto;
        uint64 nearFundedBefore;
        uint64 nearExpiresAt;
        uint64 expiresAt;
        uint256 bnbAmount;
        State state;
    }

    struct OutcomeRecord {
        bytes32 swapId;
        bytes32 nearNetworkHash;
        address depositor;
        address lp;
        bytes32 hashlock;
        bytes32 nearVaultHash;
        bytes32 nearRecipientHash;
        uint128 nearAmountYocto;
        uint256 bnbAmount;
        uint64 bscExpiresAt;
        uint64 nearFundedBefore;
        uint64 nearExpiresAt;
        uint8 outcome;
    }

    uint64 public constant MIN_CROSS_CHAIN_BUFFER = 1 hours;
    bytes32 public constant NEAR_NETWORK_HASH = keccak256("testnet");
    bytes32 private constant DOMAIN_TYPEHASH =
        keccak256("EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)");
    bytes32 private constant OUTCOME_TYPEHASH = keccak256(
        "NearFinalOutcome(bytes32 swapId,bytes32 nearNetworkHash,address depositor,address lp,bytes32 hashlock,bytes32 nearVaultHash,bytes32 nearRecipientHash,uint128 nearAmountYocto,uint256 bnbAmount,uint64 bscExpiresAt,uint64 nearFundedBefore,uint64 nearExpiresAt,uint8 outcome)"
    );
    bytes32 private immutable domainSeparator;
    mapping(address => bool) public isVerifier;
    mapping(bytes32 => Swap) public swaps;
    uint256 public lockedBnb;
    bool private entered;

    event Locked(bytes32 indexed swapId, address indexed depositor, address indexed lp, uint256 bnbAmount);
    event Paid(bytes32 indexed swapId, NearOutcome outcome, address indexed recipient, uint256 bnbAmount);

    constructor(address[3] memory verifiers) {
        require(block.chainid == 97 || block.chainid == 31337, "testnet only");
        for (uint256 i; i < 3; i++) {
            address v = verifiers[i];
            require(v != address(0) && !isVerifier[v], "bad verifier quorum");
            isVerifier[v] = true;
        }
        domainSeparator = keccak256(
            abi.encode(DOMAIN_TYPEHASH, keccak256("X3NearFinalityPilot"), keccak256("1"), block.chainid, address(this))
        );
    }

    modifier nonReentrant() {
        require(!entered, "reentrant");
        entered = true;
        _;
        entered = false;
    }

    function lock(
        bytes32 swapId,
        address lp,
        bytes32 hashlock,
        bytes32 nearVaultHash,
        bytes32 nearRecipientHash,
        uint128 nearAmountYocto,
        uint64 nearFundedBefore,
        uint64 nearExpiresAt,
        uint64 expiresAt
    ) external payable nonReentrant {
        require(swapId != bytes32(0) && swaps[swapId].state == State.Absent, "duplicate swap");
        require(lp != address(0) && hashlock != bytes32(0), "bad swap binding");
        require(nearVaultHash != bytes32(0) && nearRecipientHash != bytes32(0), "bad NEAR identity");
        require(nearAmountYocto > 0 && msg.value > 0, "zero assets");
        require(
            nearFundedBefore >= block.timestamp + 5 minutes && nearExpiresAt >= nearFundedBefore + 5 minutes
                && expiresAt >= nearExpiresAt + MIN_CROSS_CHAIN_BUFFER && expiresAt <= block.timestamp + 7 days,
            "bad cross-chain deadlines"
        );
        swaps[swapId] = Swap({
            depositor: msg.sender,
            lp: lp,
            hashlock: hashlock,
            nearVaultHash: nearVaultHash,
            nearRecipientHash: nearRecipientHash,
            nearAmountYocto: nearAmountYocto,
            nearFundedBefore: nearFundedBefore,
            nearExpiresAt: nearExpiresAt,
            expiresAt: expiresAt,
            bnbAmount: msg.value,
            state: State.Locked
        });
        lockedBnb += msg.value;
        emit Locked(swapId, msg.sender, lp, msg.value);
    }

    function attestationDigest(bytes32 swapId, NearOutcome outcome) public view returns (bytes32) {
        Swap storage s = swaps[swapId];
        require(s.state != State.Absent, "unknown swap");
        OutcomeRecord memory record = OutcomeRecord({
            swapId: swapId,
            nearNetworkHash: NEAR_NETWORK_HASH,
            depositor: s.depositor,
            lp: s.lp,
            hashlock: s.hashlock,
            nearVaultHash: s.nearVaultHash,
            nearRecipientHash: s.nearRecipientHash,
            nearAmountYocto: s.nearAmountYocto,
            bnbAmount: s.bnbAmount,
            bscExpiresAt: s.expiresAt,
            nearFundedBefore: s.nearFundedBefore,
            nearExpiresAt: s.nearExpiresAt,
            outcome: uint8(outcome)
        });
        bytes32 recordHash = keccak256(abi.encode(OUTCOME_TYPEHASH, record));
        return keccak256(abi.encodePacked("\x19\x01", domainSeparator, recordHash));
    }

    function settle(bytes32 swapId, NearOutcome outcome, bytes[] calldata signatures) external nonReentrant {
        Swap storage s = swaps[swapId];
        require(s.state == State.Locked, "not locked");
        // No deadline on a proven CLAIMED outcome: a late but finalized proof
        // must win over a potential refund. Both refund paths need expiry
        // AND an authenticated final NEAR terminal outcome certificate.
        // NotFunded requires irrevocably closed NEAR funding window state;
        // this contract cannot independently authenticate that fact.
        if (outcome == NearOutcome.Refunded) {
            require(block.timestamp >= s.nearExpiresAt, "NEAR refund window still open");
            require(block.timestamp >= s.expiresAt, "refund too soon");
        } else if (outcome == NearOutcome.NotFunded) {
            require(block.timestamp >= s.nearFundedBefore, "NEAR funding window still open");
            require(block.timestamp >= s.expiresAt, "refund too soon");
        }
        require(signatures.length == 2, "need two signatures");
        bytes32 digest = attestationDigest(swapId, outcome);
        address firstSigner = _recover(digest, signatures[0]);
        address secondSigner = _recover(digest, signatures[1]);
        require(firstSigner != secondSigner && isVerifier[firstSigner] && isVerifier[secondSigner], "invalid quorum");
        s.state = outcome == NearOutcome.Claimed ? State.Released : State.Refunded;
        lockedBnb -= s.bnbAmount;
        address recipient = outcome == NearOutcome.Claimed ? s.lp : s.depositor;
        (bool ok,) = payable(recipient).call{value: s.bnbAmount}("");
        require(ok, "payout failed");
        emit Paid(swapId, outcome, recipient, s.bnbAmount);
    }

    function _recover(bytes32 digest, bytes calldata sig) internal pure returns (address) {
        require(sig.length == 65, "bad sig length");
        bytes32 r;
        bytes32 s;
        uint8 v;
        assembly {
            r := calldataload(sig.offset)
            s := calldataload(add(sig.offset, 32))
            v := byte(0, calldataload(add(sig.offset, 64)))
        }
        require(v == 27 || v == 28, "bad recovery id");
        // secp256k1n/2: reject signature malleability.
        require(
            uint256(s) > 0 && uint256(s) <= 0x7fffffffffffffffffffffffffffffff5d576e7357a4501ddfe92f46681b20a0,
            "malleable sig"
        );
        address signer = ecrecover(digest, v, r, s);
        require(signer != address(0), "bad signer");
        return signer;
    }

    receive() external payable {
        revert("use lock");
    }
}
