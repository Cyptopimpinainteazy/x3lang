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
    uint64 constant WAIT = 7200;
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
}
