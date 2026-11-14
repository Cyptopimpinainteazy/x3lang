// SPDX-License-Identifier: MIT
pragma solidity 0.8.24;

import "../src/NearSettledBnbEscrow.sol";
import "./NearSettlementVerifierDouble.sol";

interface SettledVm {
    function warp(uint256) external;
    function deal(address, uint256) external;
    function prank(address) external;
    function expectRevert() external;
    function expectRevert(bytes calldata) external;
    function chainId(uint256) external;
}

/// @dev LP payout address that rejects BNB (withdrawal callback failure).
contract RejectingReceiver {
    function withdrawFrom(NearSettledBnbEscrow escrow, address payable to) external {
        escrow.withdraw(to);
    }

    receive() external payable {
        revert("no thanks");
    }
}

/// @dev LP payout contract that tries to re-enter on receipt of BNB.
contract ReentrantReceiver {
    NearSettledBnbEscrow public escrow;
    bytes32 public windowId;
    bytes public proof;
    uint256 public reentryAttempts;
    uint256 public reentrySuccesses;

    function arm(NearSettledBnbEscrow e, bytes32 id, bytes calldata p) external {
        escrow = e;
        windowId = id;
        proof = p;
    }

    function pull() external {
        escrow.withdraw(payable(address(this)));
    }

    receive() external payable {
        reentryAttempts += 1;
        try escrow.withdraw(payable(address(this))) {
            reentrySuccesses += 1;
        } catch {}
        try escrow.settle(windowId, 2, proof) {
            reentrySuccesses += 1;
        } catch {}
    }
}

/**
 * Issue #42 adversarial matrix for the NEAR-finality-bound BNB escrow.
 * The NEAR chain is modelled by `NearSettlementVerifierDouble`; see its header
 * for what that does and does not prove.
 */
contract NearSettledBnbEscrowTest {
    SettledVm constant vm = SettledVm(address(uint160(uint256(keccak256("hevm cheat code")))));

    bytes32 constant GENESIS = keccak256("near-testnet-genesis-under-test");
    bytes32 constant CODE_HASH = keccak256("x3-near-native-htlc wasm under test");
    string constant VAULT = "x3-vault.testnet";
    bytes32 constant SECRET = 0x0303030303030303030303030303030303030303030303030303030303030303;
    uint64 constant DEADLINE_NS = 1_700_000_600_000_000_000;
    uint64 constant EXPIRY_NS = 1_700_001_200_000_000_000;
    uint128 constant NEAR_ONE = 1e24;
    address constant DEPOSITOR = address(0xA11CE);
    address constant LP = address(0xBEEF);
    address constant RELAYER = address(0xCAFE);

    NearSettlementVerifierDouble near;
    NearSettledBnbEscrow escrow;
    uint64 height = 100;

    function setUp() public {
        vm.warp(1_700_000_000);
        near = new NearSettlementVerifierDouble(GENESIS);
        escrow = new NearSettledBnbEscrow(address(near), GENESIS, VAULT, CODE_HASH);
        vm.deal(DEPOSITOR, 100 ether);
    }

    // ------------------------------------------------------------- helpers

    function _terms(bytes32 salt) internal pure returns (NearSettledBnbEscrow.LockTerms memory) {
        return NearSettledBnbEscrow.LockTerms({
            lp: LP,
            salt: salt,
            nearInitiator: "lp.testnet",
            nearRecipient: "alice.testnet",
            hashlock: sha256(abi.encodePacked(SECRET)),
            nearAmountYocto: NEAR_ONE,
            fundingDeadlineNs: DEADLINE_NS,
            nearExpiryNs: EXPIRY_NS
        });
    }

    function _windowId(address depositor, uint256 amount, NearSettledBnbEscrow.LockTerms memory t)
        internal
        view
        returns (bytes32)
    {
        return escrow.expectedWindowId(depositor, amount, t);
    }

    /// NEAR executes a vault receipt emitting `outcome` for `windowId`.
    function _near(bytes32 windowId, uint8 outcome, bool success, bool finalize) internal returns (bytes memory p) {
        height += 1;
        p = near.execute(height, VAULT, CODE_HASH, success, escrow.settlementCommitment(windowId, outcome));
        if (finalize) near.finalize(height);
    }

    function _lock(bytes32 salt) internal returns (bytes32 windowId) {
        NearSettledBnbEscrow.LockTerms memory t = _terms(salt);
        windowId = _windowId(DEPOSITOR, 1 ether, t);
        bytes memory reg = _near(windowId, 1, true, true);
        vm.prank(DEPOSITOR);
        escrow.lock{value: 1 ether}(t, reg);
    }

    function _settle(bytes32 windowId, uint8 outcome, bytes memory p) internal {
        vm.prank(RELAYER);
        escrow.settle(windowId, outcome, p);
    }

    function _trySettle(bytes32 windowId, uint8 outcome, bytes memory p) internal returns (bool ok) {
        vm.prank(RELAYER);
        try escrow.settle(windowId, outcome, p) {
            ok = true;
        } catch {
            ok = false;
        }
    }

    function _solvent() internal view {
        require(address(escrow).balance >= escrow.lockedBnb() + escrow.owedBnb(), "escrow insolvent");
    }

    // --------------------------------------------- cross-language encoding

    /// Same vector as near/src/lib.rs and test_settlement.py.
    function testSettlementEncodingMatchesCrossLanguageVector() public view {
        bytes32 binding = escrow.sourceBindingFor(
            31337,
            address(0xE5C40),
            address(0xA11CE),
            address(0xBEEF),
            1 ether,
            0x1111111111111111111111111111111111111111111111111111111111111111
        );
        require(binding == 0xf9ceb35c05cbdd513aa7342a5ba33dd8538a9580668cea14017b5ef7f91d05aa, "source binding");
        require(
            sha256(abi.encodePacked(SECRET)) == 0x648aa5c579fb30f38af744d97d6ec840c7a91277a499a0d780f3e7314eca090b,
            "hashlock"
        );
        bytes32 id = escrow.windowIdFor(
            "x3-vault.testnet",
            "lp.testnet",
            "alice.testnet",
            sha256(abi.encodePacked(SECRET)),
            NEAR_ONE,
            DEADLINE_NS,
            EXPIRY_NS,
            binding
        );
        require(id == 0x9d66ddfe92e56e993dd87e2f7ca53b87e39ceb35325cc44b404c0a8eb87de110, "window id");
        require(
            escrow.settlementCommitment(id, 1) == 0x4b7b258b6db8ae6b3979c0ef505ecf625c0ab59727439d980ce017b01fa04808,
            "registered"
        );
        require(
            escrow.settlementCommitment(id, 2) == 0xc956f53937ecff906f42672b636a1b8216fd7beade6dd7eeaaa2074e52d9177e,
            "claimed"
        );
        require(
            escrow.settlementCommitment(id, 3) == 0x561cc941f2d1e81c86d2d428753acae16de924e836c2d9b589dbbc4a303bbb17,
            "refunded"
        );
        require(
            escrow.settlementCommitment(id, 4) == 0xe440e5dd802c852009e6e7315b05e404404d8f6f861d4a2f114ee9014dde9dff,
            "not funded"
        );
    }

    // --------------------------------------------------------- happy paths

    function testFinalNearClaimPaysLpViaAnyRelayerWithoutDepositor() public {
        bytes32 id = _lock("s1");
        _settle(id, 2, _near(id, 2, true, true));
        require(escrow.credits(LP) == 1 ether && escrow.lockedBnb() == 0, "LP not credited");
        vm.prank(LP);
        escrow.withdraw(payable(LP));
        require(LP.balance == 1 ether, "LP not paid");
        _solvent();
    }

    function testFinalNearRefundReturnsBnbToDepositor() public {
        bytes32 id = _lock("s1");
        _settle(id, 3, _near(id, 3, true, true));
        require(escrow.credits(DEPOSITOR) == 1 ether && escrow.credits(LP) == 0, "depositor not credited");
        _solvent();
    }

    /// Omitted destination funding: the LP never funds NEAR.
    function testOmittedNearFundingRefundsDepositorOnlyViaNotFunded() public {
        bytes32 id = _lock("s1");
        vm.warp(block.timestamp + 365 days);
        require(!_trySettle(id, 3, hex""), "refund without evidence");
        _settle(id, 4, _near(id, 4, true, true));
        require(escrow.credits(DEPOSITOR) == 1 ether, "not refunded");
    }

    // ----------------------------------------- secret publication / failed claim

    /// Core issue #42 attack: Alice's NEAR claim FAILS (expired, front-run,
    /// out of gas) but its arguments publish the preimage. Even a log line in
    /// the failed receipt must not pay the LP. NEAR then refunds; BNB returns.
    function testLeakedSecretFromFailedNearClaimCannotPayLp() public {
        bytes32 id = _lock("s1");
        bytes memory failedClaim = _near(id, 2, false, true);
        require(!_trySettle(id, 2, failedClaim), "failed NEAR claim paid the LP");
        require(escrow.credits(LP) == 0 && escrow.lockedBnb() == 1 ether, "BNB moved");
        _settle(id, 3, _near(id, 3, true, true));
        require(escrow.credits(DEPOSITOR) == 1 ether && escrow.credits(LP) == 0, "wrong payee");
    }

    /// No entry point of the escrow takes a preimage: knowing SECRET buys
    /// nothing. Calling the legacy claim(bytes32,bytes32) selector reverts.
    function testPublishedSecretHasNoBscEntryPoint() public {
        bytes32 id = _lock("s1");
        vm.prank(LP);
        (bool ok,) = address(escrow).call(abi.encodeWithSignature("claim(bytes32,bytes32)", id, SECRET));
        require(!ok, "preimage entry point exists");
        vm.prank(LP);
        (ok,) = address(escrow).call(abi.encodeWithSignature("ownerRelease(bytes32,bytes32)", id, SECRET));
        require(!ok, "preimage entry point exists");
        require(escrow.lockedBnb() == 1 ether, "BNB moved");
    }

    // --------------------------------------------- pending / stale / reorged

    function testPendingNearClaimCannotSettleUntilFinal() public {
        bytes32 id = _lock("s1");
        bytes memory pending = _near(id, 2, true, false);
        require(!_trySettle(id, 2, pending), "pending NEAR claim paid the LP");
        near.finalize(height);
        _settle(id, 2, pending);
        require(escrow.credits(LP) == 1 ether, "final claim did not pay");
    }

    function testReorgedOutNearClaimIsRejected() public {
        bytes32 id = _lock("s1");
        bytes memory orphan = _near(id, 2, true, false);
        near.reorg(height);
        near.finalize(height);
        require(!_trySettle(id, 2, orphan), "orphaned NEAR block paid the LP");
        // After the reorg NEAR re-executes; only the canonical outcome counts.
        _settle(id, 3, _near(id, 3, true, true));
        require(escrow.credits(DEPOSITOR) == 1 ether, "depositor not refunded");
    }

    function testRegistrationProofIsNotATerminalOutcome() public {
        NearSettledBnbEscrow.LockTerms memory t = _terms("s1");
        bytes32 id = _windowId(DEPOSITOR, 1 ether, t);
        bytes memory reg = _near(id, 1, true, true);
        vm.prank(DEPOSITOR);
        escrow.lock{value: 1 ether}(t, reg);
        require(!_trySettle(id, 1, reg), "registration settled the swap");
        require(!_trySettle(id, 2, reg), "registration proof accepted as claim");
        require(!_trySettle(id, 5, reg), "unknown outcome accepted");
        require(!_trySettle(id, 0, reg), "zero outcome accepted");
    }

    function testEvidenceForOtherVaultCodeOrChainIsRejected() public {
        bytes32 id = _lock("s1");
        bytes32 claimed = escrow.settlementCommitment(id, 2);
        height += 1;
        bytes memory otherVault = near.execute(height, "evil-vault.testnet", CODE_HASH, true, claimed);
        height += 1;
        bytes memory otherCode = near.execute(height, VAULT, keccak256("upgraded wasm"), true, claimed);
        near.finalize(height);
        require(!_trySettle(id, 2, otherVault), "foreign vault accepted");
        require(!_trySettle(id, 2, otherCode), "redeployed vault code accepted");

        // Same NEAR evidence, escrow pinned to another NEAR chain.
        NearSettlementVerifierDouble otherChain = new NearSettlementVerifierDouble(keccak256("mainnet"));
        vm.expectRevert(bytes("verifier chain mismatch"));
        new NearSettledBnbEscrow(address(otherChain), GENESIS, VAULT, CODE_HASH);
    }

    // ---------------------------------------------------- replay / cross-swap

    /// Same secret and hashlock reused for two swaps: a final claim of one
    /// window cannot settle the other; replaying a proof cannot double-pay.
    function testSameSecretReplayAcrossSwapsAndProofReplayRejected() public {
        bytes32 a = _lock("swap-a");
        bytes32 b = _lock("swap-b");
        require(a != b, "distinct salts must give distinct windows");
        bytes memory claimA = _near(a, 2, true, true);
        require(!_trySettle(b, 2, claimA), "cross-swap replay paid the LP");
        _settle(a, 2, claimA);
        require(!_trySettle(a, 2, claimA), "proof replay");
        require(!_trySettle(a, 3, _near(a, 3, true, true)), "second outcome after settlement");
        _settle(b, 3, _near(b, 3, true, true));
        require(escrow.credits(LP) == 1 ether && escrow.credits(DEPOSITOR) == 1 ether, "wrong split");
        _solvent();
    }

    function testWindowCannotBeLockedTwice() public {
        NearSettledBnbEscrow.LockTerms memory t = _terms("s1");
        bytes32 id = _windowId(DEPOSITOR, 1 ether, t);
        bytes memory reg = _near(id, 1, true, true);
        vm.prank(DEPOSITOR);
        escrow.lock{value: 1 ether}(t, reg);
        vm.prank(DEPOSITOR);
        vm.expectRevert(bytes("window already used"));
        escrow.lock{value: 1 ether}(t, reg);
    }

    // ------------------------------------------------- lock-time binding

    function testLockRequiresFinalRegistrationOfExactTerms() public {
        NearSettledBnbEscrow.LockTerms memory t = _terms("s1");
        bytes32 id = _windowId(DEPOSITOR, 1 ether, t);
        bytes memory pendingReg = _near(id, 1, true, false);
        vm.prank(DEPOSITOR);
        vm.expectRevert(bytes("NEAR outcome not verified"));
        escrow.lock{value: 1 ether}(t, pendingReg);
        near.finalize(height);

        // Partial source execution: less BNB than the registered window binds.
        vm.prank(DEPOSITOR);
        vm.expectRevert(bytes("NEAR outcome not verified"));
        escrow.lock{value: 0.5 ether}(t, pendingReg);

        // Different NEAR terms than were registered.
        NearSettledBnbEscrow.LockTerms memory more = _terms("s1");
        more.nearAmountYocto = NEAR_ONE - 1;
        vm.prank(DEPOSITOR);
        vm.expectRevert(bytes("NEAR outcome not verified"));
        escrow.lock{value: 1 ether}(more, pendingReg);

        vm.prank(DEPOSITOR);
        escrow.lock{value: 1 ether}(t, pendingReg);
        require(escrow.lockedBnb() == 1 ether, "lock failed");
    }

    /// A front-runner copying Alice's registered terms derives a different
    /// window id (depositor is bound), so it cannot occupy Alice's window.
    function testFrontRunnerCannotUseAnotherDepositorsWindow() public {
        NearSettledBnbEscrow.LockTerms memory t = _terms("s1");
        bytes32 id = _windowId(DEPOSITOR, 1 ether, t);
        bytes memory reg = _near(id, 1, true, true);
        vm.deal(RELAYER, 1 ether);
        vm.prank(RELAYER);
        vm.expectRevert(bytes("NEAR outcome not verified"));
        escrow.lock{value: 1 ether}(t, reg);
        vm.prank(DEPOSITOR);
        escrow.lock{value: 1 ether}(t, reg);
    }

    // ------------------------------------------------- timelock boundaries

    /// There is no BSC clock in the payout rule. A NEAR claim that finalized
    /// just before NEAR expiry, relayed arbitrarily late, still pays the LP;
    /// the depositor has no time-based refund to race it with.
    function testLateFinalClaimStillPaysLpAndNoTimeoutRefundExists() public {
        bytes32 id = _lock("s1");
        bytes memory claim = _near(id, 2, true, true);
        vm.warp(block.timestamp + 3650 days);
        vm.prank(DEPOSITOR);
        (bool ok,) = address(escrow).call(abi.encodeWithSignature("refund(bytes32)", id));
        require(!ok, "time-based refund exists");
        require(!_trySettle(id, 3, hex""), "refund without NEAR evidence");
        _settle(id, 2, claim);
        require(escrow.credits(LP) == 1 ether, "late claim lost");
    }

    // ------------------------------------------- withdrawal callback failure

    function testWithdrawalCallbackFailureKeepsCreditAndCanRedirect() public {
        RejectingReceiver lpContract = new RejectingReceiver();
        NearSettledBnbEscrow.LockTerms memory t = _terms("s1");
        t.lp = address(lpContract);
        bytes32 id = _windowId(DEPOSITOR, 1 ether, t);
        bytes memory reg = _near(id, 1, true, true);
        vm.prank(DEPOSITOR);
        escrow.lock{value: 1 ether}(t, reg);
        _settle(id, 2, _near(id, 2, true, true));

        vm.expectRevert(bytes("withdraw transfer failed"));
        lpContract.withdrawFrom(escrow, payable(address(lpContract)));
        require(escrow.credits(address(lpContract)) == 1 ether, "credit lost on failed callback");
        require(escrow.owedBnb() == 1 ether, "liability lost");
        _solvent();

        lpContract.withdrawFrom(escrow, payable(LP));
        require(LP.balance == 1 ether && escrow.credits(address(lpContract)) == 0, "redirect failed");
        _solvent();
    }

    // ---------------------------------------------------------- reentrancy

    function testReentrantWithdrawAndSettleCannotDoublePay() public {
        ReentrantReceiver attacker = new ReentrantReceiver();
        NearSettledBnbEscrow.LockTerms memory t = _terms("s1");
        t.lp = address(attacker);
        bytes32 id = _windowId(DEPOSITOR, 1 ether, t);
        bytes memory reg = _near(id, 1, true, true);
        vm.prank(DEPOSITOR);
        escrow.lock{value: 1 ether}(t, reg);
        // A second swap whose claim proof the attacker tries to settle mid-withdraw.
        bytes32 other = _lock("other");
        bytes memory otherClaim = _near(other, 2, true, true);
        _settle(id, 2, _near(id, 2, true, true));
        attacker.arm(escrow, other, otherClaim);
        attacker.pull();
        require(attacker.reentryAttempts() == 1, "receive not reached");
        require(attacker.reentrySuccesses() == 0, "reentrancy succeeded");
        require(address(attacker).balance == 1 ether, "wrong payout");
        require(escrow.credits(address(attacker)) == 0, "credit not cleared");
        (,,, NearSettledBnbEscrow.State st) = escrow.swaps(other);
        require(st == NearSettledBnbEscrow.State.Locked, "nested settle happened");
        _solvent();
    }

    // ---------------------------------------------- verifier fail-closed

    function _escrowWith(address v) internal returns (NearSettledBnbEscrow e, bytes32 id) {
        e = new NearSettledBnbEscrow(v, GENESIS, VAULT, CODE_HASH);
        NearSettledBnbEscrow.LockTerms memory t = _terms("s1");
        id = e.expectedWindowId(DEPOSITOR, 1 ether, t);
        vm.prank(DEPOSITOR);
        (bool ok,) = address(e).call{value: 1 ether}(abi.encodeCall(e.lock, (t, abi.encode(bytes32("x")))));
        require(!ok, "malformed verifier admitted a lock");
    }

    function testMalformedVerifiersFailClosed() public {
        _escrowWith(address(new RevertingVerifier(GENESIS)));
        _escrowWith(address(new BoolTrueVerifier(GENESIS)));
        _escrowWith(address(new MalformedMagicVerifier(GENESIS, 0)));
        _escrowWith(address(new MalformedMagicVerifier(GENESIS, 1)));
        _escrowWith(address(new MalformedMagicVerifier(GENESIS, 2)));
        StatefulVerifier stateful = new StatefulVerifier(GENESIS);
        _escrowWith(address(stateful));
        require(stateful.calls() == 0, "verifier mutated state");
    }

    function testConstructorRejectsMissingVerifierMainnetAndNonTestnetVault() public {
        vm.expectRevert(bytes("verifier must be a contract"));
        new NearSettledBnbEscrow(address(0x1234), GENESIS, VAULT, CODE_HASH);
        vm.expectRevert(bytes("testnet NEAR vault only"));
        new NearSettledBnbEscrow(address(near), GENESIS, "x3-vault.near", CODE_HASH);
        vm.expectRevert(bytes("zero NEAR identity"));
        new NearSettledBnbEscrow(address(near), GENESIS, VAULT, bytes32(0));
        vm.chainId(56);
        vm.expectRevert(bytes("testnet only"));
        new NearSettledBnbEscrow(address(near), GENESIS, VAULT, CODE_HASH);
    }

    function testDirectBnbTransferRejected() public {
        vm.prank(DEPOSITOR);
        (bool ok,) = address(escrow).call{value: 1 ether}("");
        require(!ok, "escrow accepted untracked BNB");
    }
}
