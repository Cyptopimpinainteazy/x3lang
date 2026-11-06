// SPDX-License-Identifier: MIT
pragma solidity 0.8.24;
import "../src/AttestedBnbEscrow.sol";

interface PilotVm {
    function warp(uint256) external;
    function deal(address, uint256) external;
    function prank(address) external;
    function expectRevert() external;
    function addr(uint256) external returns (address);
    function sign(uint256, bytes32) external returns (uint8, bytes32, bytes32);
}

contract AttestedBnbEscrowTest {
    PilotVm constant vm = PilotVm(address(uint160(uint256(keccak256("hevm cheat code")))));
    AttestedBnbEscrow escrow;
    bytes32 constant ID = keccak256("x3-pilot-1");
    bytes32 constant ID2 = keccak256("x3-pilot-2");
    address constant LP = address(0xBEEF);
    uint64 constant NEAR_FUNDED_BEFORE = 3600;
    uint64 constant NEAR_EXPIRY = 7200;
    uint64 constant WAIT = 10800;
    uint256 constant START = 1_700_000_000;
    receive() external payable {}

    function setUp() public {
        vm.warp(START);
        vm.deal(address(this), 10 ether);
        escrow = new AttestedBnbEscrow([vm.addr(11), vm.addr(22), vm.addr(33)]);
        _lock(ID);
    }

    function _lock(bytes32 id) internal {
        escrow.lock{value: 1 ether}(
            id,
            LP,
            keccak256("preimage commitment"),
            keccak256("vault.near"),
            keccak256("recipient.near"),
            1e24,
            uint64(block.timestamp) + NEAR_FUNDED_BEFORE,
            uint64(block.timestamp) + NEAR_EXPIRY,
            uint64(block.timestamp) + WAIT
        );
    }

    function _signature(uint256 key, bytes32 digest) internal returns (bytes memory) {
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(key, digest);
        return abi.encodePacked(r, s, v);
    }

    function _pair(bytes32 id, AttestedBnbEscrow.NearOutcome outcome) internal returns (bytes[] memory result) {
        bytes32 digest = escrow.attestationDigest(id, outcome);
        result = new bytes[](2);
        result[0] = _signature(11, digest);
        result[1] = _signature(22, digest);
    }

    function testFinalNearClaimAutomaticallyPaysLpWithoutDepositorApproval() public {
        uint256 prior = LP.balance;
        vm.prank(LP);
        escrow.settle(ID, AttestedBnbEscrow.NearOutcome.Claimed, _pair(ID, AttestedBnbEscrow.NearOutcome.Claimed));
        require(LP.balance == prior + 1 ether, "unpaid LP");
        require(escrow.lockedBnb() == 0, "liability not cleared");
    }

    function testPreimageAloneCannotRelease() public {
        bytes[] memory missingProof = new bytes[](0);
        vm.expectRevert();
        escrow.settle(ID, AttestedBnbEscrow.NearOutcome.Claimed, missingProof);
    }

    function testRefundRequiresFinalNearRefundCertificateAndBscExpiry() public {
        bytes[] memory proof = _pair(ID, AttestedBnbEscrow.NearOutcome.Refunded);
        vm.expectRevert();
        escrow.settle(ID, AttestedBnbEscrow.NearOutcome.Refunded, proof);
        vm.warp(START + WAIT);
        escrow.settle(ID, AttestedBnbEscrow.NearOutcome.Refunded, proof);
        require(escrow.lockedBnb() == 0, "refund liability still locked");
    }

    function testDepositorCannotRefundUnilaterallyAfterNearCredited() public {
        vm.warp(START + WAIT + 3600);
        bytes[] memory noProof = new bytes[](0);
        vm.expectRevert();
        escrow.settle(ID, AttestedBnbEscrow.NearOutcome.Refunded, noProof);
        escrow.settle(ID, AttestedBnbEscrow.NearOutcome.Claimed, _pair(ID, AttestedBnbEscrow.NearOutcome.Claimed));
        require(LP.balance == 1 ether, "LP not paid");
    }

    function testDuplicateSignerAndUnauthorizedSignerRejected() public {
        bytes32 digest = escrow.attestationDigest(ID, AttestedBnbEscrow.NearOutcome.Claimed);
        bytes[] memory pair = new bytes[](2);
        pair[0] = _signature(11, digest);
        pair[1] = _signature(11, digest);
        vm.expectRevert();
        escrow.settle(ID, AttestedBnbEscrow.NearOutcome.Claimed, pair);
        pair[1] = _signature(999, digest);
        vm.expectRevert();
        escrow.settle(ID, AttestedBnbEscrow.NearOutcome.Claimed, pair);
    }

    function testClaimProofCannotBeUsedAsRefundProof() public {
        vm.warp(START + WAIT);
        bytes[] memory wrongProof = _pair(ID, AttestedBnbEscrow.NearOutcome.Claimed);
        vm.expectRevert();
        escrow.settle(ID, AttestedBnbEscrow.NearOutcome.Refunded, wrongProof);
    }

    function testProofCannotBeReplayedForAnotherSwap() public {
        _lock(ID2);
        bytes[] memory wrongSwapProof = _pair(ID, AttestedBnbEscrow.NearOutcome.Claimed);
        vm.expectRevert();
        escrow.settle(ID2, AttestedBnbEscrow.NearOutcome.Claimed, wrongSwapProof);
    }

    function testQuorumOutageKeepsFundsLockedRatherThanAllowingBlindRefund() public {
        vm.warp(START + WAIT + 30 days);
        bytes[] memory insufficient = new bytes[](1);
        insufficient[0] = _signature(11, escrow.attestationDigest(ID, AttestedBnbEscrow.NearOutcome.Refunded));
        vm.expectRevert();
        escrow.settle(ID, AttestedBnbEscrow.NearOutcome.Refunded, insufficient);
        require(escrow.lockedBnb() == 1 ether, "funds escaped without finalized evidence");
    }

    function testAnyTwoAuthorizedSignersCanReleaseEvenWithoutRealNearEvidence() public {
        // Trust assumption demonstration: the BSC contract verifies signatures,
        // NOT actual NEAR receipts. Honest external verification is mandatory.
        bytes[] memory quorum = _pair(ID, AttestedBnbEscrow.NearOutcome.Claimed);
        escrow.settle(ID, AttestedBnbEscrow.NearOutcome.Claimed, quorum);
        require(LP.balance == 1 ether, "committee certificate was not decisive");
    }

    function testNotFundedCertificateRequiresBscExpiry() public {
        bytes[] memory proof = _pair(ID, AttestedBnbEscrow.NearOutcome.NotFunded);
        vm.expectRevert();
        escrow.settle(ID, AttestedBnbEscrow.NearOutcome.NotFunded, proof);
        uint256 before = address(this).balance;
        vm.warp(START + WAIT);
        escrow.settle(ID, AttestedBnbEscrow.NearOutcome.NotFunded, proof);
        require(address(this).balance == before + 1 ether, "missing funding refund failed");
        require(escrow.lockedBnb() == 0, "liability not discharged");
    }

    function testClaimCertificateCannotBeReplayedAsNotFunded() public {
        bytes[] memory claimProof = _pair(ID, AttestedBnbEscrow.NearOutcome.Claimed);
        vm.warp(START + WAIT);
        vm.expectRevert();
        escrow.settle(ID, AttestedBnbEscrow.NearOutcome.NotFunded, claimProof);
    }

    function testCannotDoubleSettleOrRefundAfterClaim() public {
        bytes[] memory claimProof = _pair(ID, AttestedBnbEscrow.NearOutcome.Claimed);
        escrow.settle(ID, AttestedBnbEscrow.NearOutcome.Claimed, claimProof);
        vm.warp(START + WAIT);
        vm.expectRevert();
        escrow.settle(ID, AttestedBnbEscrow.NearOutcome.Claimed, claimProof);
        bytes[] memory refundProof = _pair(ID, AttestedBnbEscrow.NearOutcome.Refunded);
        vm.expectRevert();
        escrow.settle(ID, AttestedBnbEscrow.NearOutcome.Refunded, refundProof);
    }

    function testRejectsUnsafeCrossChainDeadlineOrdering() public {
        bytes32 id = keccak256("unsafe-deadlines");
        vm.expectRevert();
        escrow.lock{value: 1 ether}(
            id,
            LP,
            keccak256("preimage commitment"),
            keccak256("vault.near"),
            keccak256("recipient.near"),
            1e24,
            uint64(block.timestamp) + NEAR_EXPIRY,
            uint64(block.timestamp) + NEAR_EXPIRY,
            uint64(block.timestamp) + NEAR_EXPIRY
        );
    }

    function testRejectsASwapWhereTheDepositorIsAlsoTheLp() public {
        bytes32 id = keccak256("self-dealing");
        vm.expectRevert();
        escrow.lock{value: 1 ether}(
            id,
            address(this),
            keccak256("preimage commitment"),
            keccak256("vault.near"),
            keccak256("recipient.near"),
            1e24,
            uint64(block.timestamp) + NEAR_FUNDED_BEFORE,
            uint64(block.timestamp) + NEAR_EXPIRY,
            uint64(block.timestamp) + WAIT
        );
    }

    function testAttestationBindsVaultAmountDeadlinesAndParties() public {
        AttestedBnbEscrow other = new AttestedBnbEscrow([vm.addr(11), vm.addr(22), vm.addr(33)]);
        other.lock{value: 2 ether}(
            ID,
            address(0xCAFE),
            keccak256("preimage commitment"),
            keccak256("other-vault.near"),
            keccak256("recipient.near"),
            2e24,
            uint64(block.timestamp) + NEAR_FUNDED_BEFORE,
            uint64(block.timestamp) + NEAR_EXPIRY,
            uint64(block.timestamp) + WAIT
        );
        bytes[] memory proofFromFirst = _pair(ID, AttestedBnbEscrow.NearOutcome.Claimed);
        vm.expectRevert();
        other.settle(ID, AttestedBnbEscrow.NearOutcome.Claimed, proofFromFirst);
    }

    function testDigestBindsNearExpiryAmountAndParties() public {
        bytes32 domainTypehash =
            keccak256("EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)");
        bytes32 outcomeTypehash = keccak256(
            "NearFinalOutcome(bytes32 swapId,bytes32 nearNetworkHash,address depositor,address lp,bytes32 hashlock,bytes32 nearVaultHash,bytes32 nearRecipientHash,uint128 nearAmountYocto,uint256 bnbAmount,uint64 bscExpiresAt,uint64 nearFundedBefore,uint64 nearExpiresAt,uint8 outcome)"
        );
        bytes32 domainSeparator = keccak256(
            abi.encode(domainTypehash, keccak256("X3NearFinalityPilot"), keccak256("1"), block.chainid, address(escrow))
        );
        AttestedBnbEscrow.OutcomeRecord memory substituted = AttestedBnbEscrow.OutcomeRecord({
            swapId: ID,
            nearNetworkHash: escrow.NEAR_NETWORK_HASH(),
            depositor: address(this),
            lp: address(0xCAFE),
            hashlock: keccak256("preimage commitment"),
            nearVaultHash: keccak256("vault.near"),
            nearRecipientHash: keccak256("recipient.near"),
            nearAmountYocto: 2e24,
            bnbAmount: 1 ether,
            bscExpiresAt: uint64(START + WAIT),
            nearFundedBefore: uint64(START + NEAR_FUNDED_BEFORE),
            nearExpiresAt: uint64(START + NEAR_EXPIRY + 1),
            outcome: uint8(AttestedBnbEscrow.NearOutcome.Claimed)
        });
        bytes32 digest = keccak256(
            abi.encodePacked("\x19\x01", domainSeparator, keccak256(abi.encode(outcomeTypehash, substituted)))
        );
        bytes[] memory substitutedProof = new bytes[](2);
        substitutedProof[0] = _signature(11, digest);
        substitutedProof[1] = _signature(22, digest);
        vm.expectRevert();
        escrow.settle(ID, AttestedBnbEscrow.NearOutcome.Claimed, substitutedProof);
    }

    function testRefundOutcomesCannotSettleBeforeNearestRelevantDeadline() public {
        bytes[] memory refundProof = _pair(ID, AttestedBnbEscrow.NearOutcome.Refunded);
        vm.warp(START + NEAR_EXPIRY);
        vm.expectRevert();
        escrow.settle(ID, AttestedBnbEscrow.NearOutcome.Refunded, refundProof);
        bytes[] memory notFundedProof = _pair(ID, AttestedBnbEscrow.NearOutcome.NotFunded);
        vm.warp(START + NEAR_FUNDED_BEFORE);
        vm.expectRevert();
        escrow.settle(ID, AttestedBnbEscrow.NearOutcome.NotFunded, notFundedProof);
    }
}
