// SPDX-License-Identifier: MIT
pragma solidity 0.8.24;

import "../src/NativeBnbHTLC.sol";
import "../src/NearSettledBnbEscrow.sol";
import "./legacy/LegacyPreimageHTLC.sol";
import "./NearSettlementVerifierDouble.sol";

interface RedGreenVm {
    function warp(uint256) external;
    function deal(address, uint256) external;
    function prank(address) external;
}

/**
 * Issue #42 red/green pairs. Each RED test is a deliberately PASSING
 * demonstration that an attack SUCCEEDS against an older BSC withdrawal rule;
 * its GREEN partner runs the same cross-chain scenario against
 * NearSettledBnbEscrow and shows the attack failing.
 *
 * NEAR is modelled by NearSettlementVerifierDouble (a test double). For the
 * legacy contracts NEAR state is irrelevant to the BSC rule, which is exactly
 * the defect: those rules never look at NEAR.
 */
contract Issue42RedGreenTest {
    RedGreenVm constant vm = RedGreenVm(address(uint160(uint256(keccak256("hevm cheat code")))));

    bytes32 constant GENESIS = keccak256("near-testnet-genesis-under-test");
    bytes32 constant CODE_HASH = keccak256("x3-near-native-htlc wasm under test");
    string constant VAULT = "x3-vault.testnet";
    bytes32 constant SECRET = 0x0303030303030303030303030303030303030303030303030303030303030303;
    address constant ALICE = address(0xA11CE); // BNB depositor, wants NEAR
    address constant LP = address(0xBEEF); // NEAR liquidity provider, wants BNB
    uint256 constant T0 = 1_700_000_000;
    uint64 constant NEAR_EXPIRY = uint64(T0 + 3600); // NEAR expires first
    uint64 constant BSC_EXPIRY = uint64(T0 + 7200); // legacy staggered BSC expiry

    NearSettlementVerifierDouble near;
    NearSettledBnbEscrow escrow;
    uint64 height = 10;

    function setUp() public {
        vm.warp(T0);
        vm.deal(ALICE, 10 ether);
        near = new NearSettlementVerifierDouble(GENESIS);
        escrow = new NearSettledBnbEscrow(address(near), GENESIS, VAULT, CODE_HASH);
    }

    function _nearOutcome(bytes32 windowId, uint8 outcome, bool success) internal returns (bytes memory p) {
        height += 1;
        p = near.execute(height, VAULT, CODE_HASH, success, escrow.settlementCommitment(windowId, outcome));
        near.finalize(height);
    }

    function _greenLock() internal returns (bytes32 id) {
        NearSettledBnbEscrow.LockTerms memory t = NearSettledBnbEscrow.LockTerms({
            lp: LP,
            salt: "red-green",
            nearInitiator: "lp.testnet",
            nearRecipient: "alice.testnet",
            hashlock: sha256(abi.encodePacked(SECRET)),
            nearAmountYocto: 1e24,
            fundingDeadlineNs: uint64(T0 + 600) * 1e9,
            nearExpiryNs: uint64(NEAR_EXPIRY) * 1e9
        });
        id = escrow.expectedWindowId(ALICE, 1 ether, t);
        bytes memory reg = _nearOutcome(id, 1, true);
        vm.prank(ALICE);
        escrow.lock{value: 1 ether}(t, reg);
    }

    function _trySettle(bytes32 id, uint8 outcome, bytes memory p) internal returns (bool ok) {
        vm.prank(LP);
        try escrow.settle(id, outcome, p) {
            ok = true;
        } catch {
            ok = false;
        }
    }

    // ---------------------------------------------------------------------
    // Attack 1: failed NEAR claim leaks the preimage (expiry race).
    // Alice's NEAR claim lands at NEAR expiry and fails, but its arguments
    // publish SECRET. NEAR then refunds the LP. The LP uses SECRET on BSC.
    // ---------------------------------------------------------------------

    function testRed_LegacyPreimageHtlc_LeakedSecretLetsLpTakeBothAssets() public {
        LegacyPreimageHTLC legacy = new LegacyPreimageHTLC();
        bytes32 id = keccak256("legacy-1");
        bytes32 hashlock = sha256(abi.encodePacked(SECRET)); // precompile call: before prank
        vm.prank(ALICE);
        legacy.lock{value: 1 ether}(id, LP, hashlock, BSC_EXPIRY);
        (address initiator,,,,,) = legacy.swaps(id);
        require(initiator == ALICE, "setup: Alice must be the depositor");

        vm.warp(NEAR_EXPIRY); // Alice's NEAR claim fails here, publishing SECRET.
        // LP refunds its NEAR (NEAR swap Refunded) and claims BNB with SECRET.
        vm.prank(LP);
        legacy.claim(id, SECRET);
        require(LP.balance == 1 ether, "RED expected: LP took the BNB");
        // NEAR side: LP got its NEAR back, Alice got nothing. Attack succeeded.
    }

    function testGreen_NearSettledEscrow_LeakedSecretGivesLpNothing() public {
        bytes32 id = _greenLock();
        vm.warp(NEAR_EXPIRY);
        bytes memory failedClaim = _nearOutcome(id, 2, false); // failed receipt, secret public
        require(!_trySettle(id, 2, failedClaim), "GREEN violated: failed claim paid LP");
        bytes memory nearRefund = _nearOutcome(id, 3, true); // NEAR refunds LP
        require(!_trySettle(id, 2, nearRefund), "GREEN violated: refund proof paid LP");
        require(_trySettle(id, 3, nearRefund), "refund relay failed");
        require(escrow.credits(ALICE) == 1 ether && escrow.credits(LP) == 0, "GREEN violated: wrong payee");
    }

    // ---------------------------------------------------------------------
    // Attack 2: depositor keeps both (current master NativeBnbHTLC, 03a33a9).
    // NEAR claim finalizes (Alice credited); Alice withholds ownerRelease and
    // refunds BNB after BSC expiry.
    // ---------------------------------------------------------------------

    function testRed_OwnerReleaseHtlc_DepositorWithholdsAndKeepsBothAssets() public {
        NativeBnbHTLC current = new NativeBnbHTLC();
        bytes32 id = keccak256("owner-release-1");
        bytes32 hashlock = sha256(abi.encodePacked(SECRET)); // precompile call: before prank
        vm.prank(ALICE);
        current.lock{value: 1 ether}(id, LP, hashlock, BSC_EXPIRY);
        // NEAR: Alice's claim succeeds and finalizes; SECRET is public.
        vm.prank(LP);
        try current.ownerRelease(id, SECRET) {
            revert("LP should not be able to release");
        } catch {}
        vm.warp(BSC_EXPIRY);
        uint256 before = ALICE.balance;
        vm.prank(ALICE);
        current.refund(id);
        require(ALICE.balance == before + 1 ether, "RED expected: Alice recovered BNB after NEAR credit");
        require(LP.balance == 0, "RED expected: LP unpaid");
    }

    function testGreen_NearSettledEscrow_FinalNearClaimForcesLpPayout() public {
        bytes32 id = _greenLock();
        bytes memory claim = _nearOutcome(id, 2, true);
        vm.warp(BSC_EXPIRY + 365 days);
        // Alice has no refund path: NEAR can never also be Refunded, and no
        // BSC timeout exists. Anyone relays the claim; Alice cannot veto it.
        vm.prank(ALICE);
        (bool ok,) = address(escrow).call(abi.encodeWithSignature("refund(bytes32)", id));
        require(!ok, "GREEN violated: timeout refund exists");
        require(!_trySettle(id, 3, claim), "GREEN violated: claim proof refunded Alice");
        require(_trySettle(id, 2, claim), "claim relay failed");
        require(escrow.credits(LP) == 1 ether && escrow.credits(ALICE) == 0, "GREEN violated: wrong payee");
    }

    // ---------------------------------------------------------------------
    // Attack 3: depositor releases early (phished or mistaken) before the LP
    // ever funds NEAR. Current master pays the LP on the depositor's word.
    // ---------------------------------------------------------------------

    function testRed_OwnerReleaseHtlc_EarlyReleasePaysLpWithoutNearDelivery() public {
        NativeBnbHTLC current = new NativeBnbHTLC();
        bytes32 id = keccak256("owner-release-2");
        bytes32 hashlock = sha256(abi.encodePacked(SECRET)); // precompile call: before prank
        vm.prank(ALICE);
        current.lock{value: 1 ether}(id, LP, hashlock, BSC_EXPIRY);
        vm.prank(ALICE);
        current.ownerRelease(id, SECRET);
        require(LP.balance == 1 ether, "RED expected: LP paid with no NEAR delivered");
    }

    function testGreen_NearSettledEscrow_NoOneCanReleaseWithoutNearClaim() public {
        bytes32 id = _greenLock();
        vm.prank(ALICE);
        (bool ok,) = address(escrow).call(abi.encodeWithSignature("ownerRelease(bytes32,bytes32)", id, SECRET));
        require(!ok, "GREEN violated: depositor release exists");
        require(!_trySettle(id, 2, hex""), "GREEN violated: release without evidence");
        // LP never funds; NEAR closes the window NotFunded; Alice is refunded.
        vm.warp(T0 + 600);
        require(_trySettle(id, 4, _nearOutcome(id, 4, true)), "not-funded relay failed");
        require(escrow.credits(ALICE) == 1 ether && LP.balance == 0, "GREEN violated");
    }
}
