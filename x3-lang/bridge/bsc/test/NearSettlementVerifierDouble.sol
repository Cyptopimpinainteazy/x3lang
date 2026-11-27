// SPDX-License-Identifier: MIT
pragma solidity 0.8.24;

import "../src/INearSettlementVerifier.sol";

/**
 * @title NearSettlementVerifierDouble  (TEST DOUBLE ONLY - never deploy)
 * @notice Models the OBSERVABLE CONTRACT of `INearSettlementVerifier` so the
 *         escrow's behaviour can be tested against every case the interface
 *         spec says a real verifier must reject. It authenticates nothing:
 *         the test itself writes the "NEAR chain" (blocks, finality, outcomes).
 *         A passing escrow test proves the escrow is correct GIVEN a verifier
 *         that meets the spec; it proves nothing about any real verifier.
 */
contract NearSettlementVerifierDouble is INearSettlementVerifier {
    struct NearOutcome {
        uint64 height;
        bytes32 blockHash;
        bytes32 executorHash;
        bytes32 codeHash;
        bool success;
        bytes32 commitment;
    }

    bytes32 public immutable nearGenesisHash;
    uint64 public finalizedHeight;
    mapping(uint64 => bytes32) public canonicalBlock;
    mapping(bytes32 => NearOutcome) private outcomes;
    uint256 private nonce;

    constructor(bytes32 genesis) {
        nearGenesisHash = genesis;
    }

    /// @notice Simulate NEAR executing a receipt at `height`. Returns the proof
    ///         bytes a relayer would submit. Not final until `finalize`.
    function execute(uint64 height, string calldata executor, bytes32 codeHash, bool success, bytes32 commitment)
        external
        returns (bytes memory proof)
    {
        require(height > finalizedHeight, "cannot append below final head");
        if (canonicalBlock[height] == bytes32(0)) {
            canonicalBlock[height] = keccak256(abi.encode("block", height, nonce++));
        }
        bytes32 id = keccak256(abi.encode("outcome", nonce++));
        outcomes[id] = NearOutcome({
            height: height,
            blockHash: canonicalBlock[height],
            executorHash: keccak256(bytes(executor)),
            codeHash: codeHash,
            success: success,
            commitment: commitment
        });
        return abi.encode(id);
    }

    function finalize(uint64 height) external {
        require(height >= finalizedHeight, "finality cannot regress");
        finalizedHeight = height;
    }

    /// @notice Replace the canonical block at a NOT-yet-final height.
    function reorg(uint64 height) external {
        require(height > finalizedHeight, "cannot reorg a final block");
        canonicalBlock[height] = keccak256(abi.encode("fork", height, nonce++));
    }

    function verifySettlement(
        bytes32 genesis,
        string calldata nearVault,
        bytes32 nearVaultCodeHash,
        bytes32 settlementCommitment,
        bytes calldata proof
    ) external view returns (bytes4) {
        if (genesis != nearGenesisHash || proof.length != 32) return bytes4(0);
        NearOutcome storage o = outcomes[abi.decode(proof, (bytes32))];
        if (o.height == 0 || o.height > finalizedHeight) return bytes4(0); // unknown or pending
        if (canonicalBlock[o.height] != o.blockHash) return bytes4(0); // reorged out
        if (o.executorHash != keccak256(bytes(nearVault))) return bytes4(0); // other contract
        if (o.codeHash != nearVaultCodeHash) return bytes4(0); // redeployed / wrong code
        if (!o.success) return bytes4(0); // failed receipt: logs exist but do not count
        if (o.commitment != settlementCommitment) return bytes4(0);
        return INearSettlementVerifier.verifySettlement.selector;
    }
}

/// @dev Malformed verifiers used to show the escrow fails closed.
contract RevertingVerifier {
    bytes32 public immutable nearGenesisHash;

    constructor(bytes32 g) {
        nearGenesisHash = g;
    }

    function verifySettlement(bytes32, string calldata, bytes32, bytes32, bytes calldata)
        external
        pure
        returns (bytes4)
    {
        revert("verifier down");
    }
}

/// @dev Returns ABI `true`, the classic "user-supplied boolean" verifier.
contract BoolTrueVerifier {
    bytes32 public immutable nearGenesisHash;

    constructor(bytes32 g) {
        nearGenesisHash = g;
    }

    function verifySettlement(bytes32, string calldata, bytes32, bytes32, bytes calldata) external pure returns (bool) {
        return true;
    }
}

/// @dev Returns the magic selector with dirty low-order bytes, or truncated.
contract MalformedMagicVerifier {
    bytes32 public immutable nearGenesisHash;
    uint256 public immutable mode;

    constructor(bytes32 g, uint256 m) {
        nearGenesisHash = g;
        mode = m;
    }

    function verifySettlement(bytes32, string calldata, bytes32, bytes32, bytes calldata)
        external
        view
        returns (bytes4)
    {
        bytes32 word = bytes32(INearSettlementVerifier.verifySettlement.selector);
        uint256 m = mode;
        assembly {
            switch m
            case 0 {
                mstore(0, or(word, 1))
                return(0, 32)
            }
            case 1 {
                mstore(0, word)
                return(0, 4)
            }
            default {
                mstore(0, word)
                mstore(32, 0)
                return(0, 64)
            }
        }
    }
}

/// @dev Returns the magic word but writes state: must fail under STATICCALL.
contract StatefulVerifier {
    bytes32 public immutable nearGenesisHash;
    uint256 public calls;

    constructor(bytes32 g) {
        nearGenesisHash = g;
    }

    function verifySettlement(bytes32, string calldata, bytes32, bytes32, bytes calldata) external returns (bytes4) {
        calls += 1;
        return INearSettlementVerifier.verifySettlement.selector;
    }
}
