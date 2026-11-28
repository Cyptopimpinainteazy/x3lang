// SPDX-License-Identifier: MIT
pragma solidity 0.8.24;

import "../src/X3ControlledTreasury.sol";

interface Vm {
    function warp(uint256) external;
    function chainId(uint256) external;
    function prank(address) external;
    function deal(address, uint256) external;
    function expectRevert(bytes calldata) external;
    function expectEmit(bool, bool, bool, bool) external;
    function readFile(string calldata) external view returns (string memory);
    function parseBytes(string calldata) external pure returns (bytes memory);
    function parseBytes32(string calldata) external pure returns (bytes32);
    function assume(bool) external;
}

/// Installs the compiler's real output (test/fixtures, pinned by tools/x3ccp_fixtures.py --check)
/// into a real X3ControlledTreasury and proves each clause refuses on chain with its source line.
/// Clause numbers below are line numbers in examples/x3ccp_treasury_guard.x3.
contract X3ControlledTreasuryTest {
    Vm constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));

    // Lines in examples/x3ccp_treasury_guard.x3
    uint16 constant L_HEADER = 13;
    uint16 constant L_CALLERS = 16;
    uint16 constant L_WITHDRAW = 17;
    uint16 constant L_MAX = 19;
    uint16 constant L_MIN = 20;
    uint16 constant L_TO_IN = 21;
    uint16 constant L_RATE_SUM = 22;
    uint16 constant L_RATE_CALLS = 23;
    uint16 constant L_URGENT = 24;
    uint16 constant L_EXPIRES = 25;

    address constant AGENT = 0x1111111111111111111111111111111111111111;
    address constant OPS = 0x3333333333333333333333333333333333333333;
    address constant STORE = 0x2222222222222222222222222222222222222222;
    address constant BOB = 0x4444444444444444444444444444444444444444;
    address constant MALLORY = 0x6666666666666666666666666666666666666666;

    uint256 constant T0 = 1_800_000_000; // 2027-01-15, before the 2030-01-01 expiry
    uint256 constant EXPIRY = 1_893_456_000; // 2030-01-01T00:00:00Z

    X3ControlledTreasury t;
    bytes policy;
    bytes32 policyHash;

    function setUp() public {
        vm.warp(T0);
        t = new X3ControlledTreasury();
        require(address(t) == 0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f, "fixture target address moved");
        vm.deal(address(t), 10 ether);
        policy = vm.parseBytes(_trim(vm.readFile("test/fixtures/treasury_guard.hex")));
        policyHash = vm.parseBytes32(_trim(vm.readFile("test/fixtures/treasury_guard.hash")));
    }

    // ------------------------------------------------------------------ helpers

    function _trim(string memory s) internal pure returns (string memory) {
        bytes memory b = bytes(s);
        uint256 n = b.length;
        while (n > 0 && (b[n - 1] == 0x0a || b[n - 1] == 0x0d || b[n - 1] == 0x20)) n--;
        bytes memory out = new bytes(n);
        for (uint256 i = 0; i < n; i++) {
            out[i] = b[i];
        }
        return string(out);
    }

    function _refused(uint16 clause, uint8 code) internal pure returns (bytes memory) {
        return abi.encodeWithSelector(X3Controlled.X3Refused.selector, clause, code);
    }

    function _bad(uint8 reason) internal pure returns (bytes memory) {
        return abi.encodeWithSelector(X3Controlled.X3BadPolicy.selector, reason);
    }

    function _install() internal {
        t.installPolicy(policy);
    }

    function _withdraw(address caller, address to, uint256 amount) internal {
        vm.prank(caller);
        t.withdraw(to, amount);
    }

    function _expectWithdrawRefused(address caller, address to, uint256 amount, uint16 clause, uint8 code) internal {
        vm.expectRevert(_refused(clause, code));
        vm.prank(caller);
        t.withdraw(to, amount);
    }

    function _eq(uint256 a, uint256 b, string memory why) internal pure {
        require(a == b, why);
    }

    // ------------------------------------------------------------------ install

    function test_install_records_compiler_hash_and_epoch() public {
        _install();
        require(t.x3PolicyHash() == policyHash, "on-chain hash != compiler hash");
        require(t.x3PolicyHash() == keccak256(policy), "hash is keccak256(policy bytes)");
        require(t.x3Active(), "policy active");
        _eq(t.x3Epoch(), 1, "first epoch");
        _eq(t.x3ExpiresAt(), EXPIRY, "expiry from source");
        require(t.x3IsCaller(AGENT) && t.x3IsCaller(OPS) && !t.x3IsCaller(MALLORY), "caller set");
    }

    function test_only_owner_installs_and_revokes() public {
        vm.expectRevert(abi.encodeWithSelector(X3Controlled.X3NotOwner.selector));
        vm.prank(AGENT);
        t.installPolicy(policy);
        _install();
        vm.expectRevert(abi.encodeWithSelector(X3Controlled.X3NotOwner.selector));
        vm.prank(AGENT);
        t.revokePolicy();
    }

    function test_install_rejects_wrong_chain() public {
        vm.chainId(84532);
        vm.expectRevert(_bad(3));
        t.installPolicy(policy);
    }

    function test_install_rejects_other_target() public {
        X3ControlledTreasury other = new X3ControlledTreasury();
        vm.expectRevert(_bad(4));
        other.installPolicy(policy);
    }

    function test_install_rejects_wrong_owner_bytes() public {
        bytes memory p = policy;
        p[33] = bytes1(uint8(p[33]) ^ 0x01); // first byte of the owner field
        vm.expectRevert(_bad(5));
        t.installPolicy(p);
    }

    function test_install_rejects_bad_magic_and_version() public {
        bytes memory p = policy;
        p[0] = "Y";
        vm.expectRevert(_bad(1));
        t.installPolicy(p);
        p = policy;
        p[4] = 0x02;
        vm.expectRevert(_bad(2));
        t.installPolicy(p);
    }

    function test_install_rejects_already_expired() public {
        vm.warp(EXPIRY);
        vm.expectRevert(_bad(6));
        t.installPolicy(policy);
    }

    function test_install_rejects_truncated_and_trailing() public {
        bytes memory shortP = new bytes(policy.length - 1);
        for (uint256 i = 0; i < shortP.length; i++) {
            shortP[i] = policy[i];
        }
        vm.expectRevert(_bad(11));
        t.installPolicy(shortP);
        vm.expectRevert(_bad(12));
        t.installPolicy(abi.encodePacked(policy, bytes1(0x00)));
    }

    function test_install_rejects_duplicate_caller() public {
        bytes memory p = policy;
        // callers start at byte 68 (4 magic + 1 version + 8 chain + 20 target + 20 owner + 8 expiry +
        // 6 lines + 1 count); copy caller 0 over caller 1.
        for (uint256 i = 0; i < 20; i++) {
            p[68 + 20 + i] = p[68 + i];
        }
        vm.expectRevert(_bad(8));
        t.installPolicy(p);
    }

    // ------------------------------------------------------------------ fail closed

    function test_no_policy_refuses_everything() public {
        _expectWithdrawRefused(AGENT, STORE, 5_000, 0, 1);
    }

    function test_revoke_refuses_everything() public {
        _install();
        t.revokePolicy();
        require(!t.x3Active(), "revoked");
        _expectWithdrawRefused(AGENT, STORE, 5_000, 0, 1);
    }

    function test_unlisted_function_is_refused_by_default() public {
        _install();
        vm.expectRevert(_refused(L_HEADER, 5));
        vm.prank(AGENT);
        t.sweep(STORE);
        _eq(address(t).balance, 10 ether, "nothing swept");
    }

    // ------------------------------------------------------------------ allowed path

    function test_allowed_withdraw_moves_eth_and_emits() public {
        _install();
        uint256 before = STORE.balance;
        vm.expectEmit(true, true, true, false);
        emit X3Controlled.X3Allowed(1, X3ControlledTreasury.withdraw.selector, AGENT);
        _withdraw(AGENT, STORE, 50_000_000);
        _eq(STORE.balance - before, 50_000_000, "paid");
        _eq(t.x3Usage(X3ControlledTreasury.withdraw.selector, 3, block.timestamp), 50_000_000, "sum usage");
        _eq(t.x3Usage(X3ControlledTreasury.withdraw.selector, 4, block.timestamp), 1, "call usage");
    }

    // ------------------------------------------------------------------ each clause refuses with its line

    function test_caller_clause() public {
        _install();
        _expectWithdrawRefused(MALLORY, STORE, 5_000, L_CALLERS, 4);
    }

    function test_max_clause() public {
        _install();
        _expectWithdrawRefused(AGENT, STORE, 50_000_001, L_MAX, 7);
    }

    function test_min_clause() public {
        _install();
        _expectWithdrawRefused(AGENT, STORE, 999, L_MIN, 8);
    }

    function test_recipient_clause() public {
        _install();
        _expectWithdrawRefused(AGENT, MALLORY, 5_000, L_TO_IN, 10);
    }

    function test_rate_sum_clause_and_window_rollover() public {
        _install();
        _withdraw(AGENT, STORE, 50_000_000);
        _withdraw(OPS, BOB, 50_000_000);
        _expectWithdrawRefused(AGENT, STORE, 30_000_000, L_RATE_SUM, 11);
        _withdraw(AGENT, STORE, 20_000_000); // exactly 120M: allowed
        vm.warp((block.timestamp / 1 days + 1) * 1 days + 1); // next fixed day window
        _withdraw(AGENT, STORE, 50_000_000);
    }

    function test_rate_calls_clause_and_refusal_consumes_nothing() public {
        _install();
        _withdraw(AGENT, STORE, 1_000);
        _expectWithdrawRefused(AGENT, MALLORY, 1_000, L_TO_IN, 10); // refused: counts nothing
        _withdraw(AGENT, STORE, 1_000);
        _withdraw(OPS, BOB, 1_000);
        _expectWithdrawRefused(AGENT, STORE, 1_000, L_RATE_CALLS, 12);
        vm.warp((block.timestamp / 1 hours + 1) * 1 hours);
        _withdraw(AGENT, STORE, 1_000);
    }

    function test_bool_eq_clause() public {
        _install();
        vm.expectRevert(_refused(L_URGENT, 9));
        vm.prank(OPS);
        t.setMemo(bytes32("pay rent"), true);
        vm.prank(OPS);
        t.setMemo(bytes32("pay rent"), false);
        require(t.memo() == bytes32("pay rent") && !t.memoUrgent(), "memo set");
    }

    function test_expiry_clause() public {
        _install();
        vm.warp(EXPIRY);
        _expectWithdrawRefused(AGENT, STORE, 5_000, L_EXPIRES, 3);
    }

    function test_wrong_chain_after_install() public {
        _install();
        vm.chainId(84532);
        _expectWithdrawRefused(AGENT, STORE, 5_000, L_HEADER, 2);
    }

    // ------------------------------------------------------------------ calldata strictness

    function _raw(address caller, bytes memory data) internal returns (bool ok, bytes memory ret) {
        vm.prank(caller);
        (ok, ret) = address(t).call(data);
    }

    /// Solidity's ABI decoder rejects a dirty word before any modifier runs, so the revert data is
    /// empty, not X3Refused. x3ccp.predict reports this case with `abi_decoder_first`.
    function test_dirty_address_word_is_refused_by_abi_decoder() public {
        _install();
        bytes memory data = abi.encodeWithSelector(X3ControlledTreasury.withdraw.selector, STORE, uint256(5_000));
        data[4] = 0x01; // set a high byte of the address word
        uint256 before = STORE.balance;
        (bool ok, bytes memory ret) = _raw(AGENT, data);
        require(!ok && ret.length == 0, "dirty address refused by decoder");
        _eq(STORE.balance, before, "nothing paid");
        _eq(t.x3Usage(X3ControlledTreasury.withdraw.selector, 4, block.timestamp), 0, "no usage");
    }

    function test_trailing_calldata_is_bad_calldata() public {
        _install();
        bytes memory data = abi.encodePacked(
            abi.encodeWithSelector(X3ControlledTreasury.withdraw.selector, STORE, uint256(5_000)), bytes32(0)
        );
        (bool ok, bytes memory ret) = _raw(AGENT, data);
        require(!ok && keccak256(ret) == keccak256(_refused(L_WITHDRAW, 6)), "trailing data refused");
    }

    function test_non_boolean_word_is_refused_by_abi_decoder() public {
        _install();
        bytes memory data = abi.encodeWithSelector(X3ControlledTreasury.setMemo.selector, bytes32("x"), uint256(2));
        (bool ok, bytes memory ret) = _raw(OPS, data);
        require(!ok && ret.length == 0, "bool 2 refused by decoder");
        require(t.memo() == bytes32(0), "memo unchanged");
    }

    // ------------------------------------------------------------------ replacing a policy

    function test_new_policy_new_epoch_new_limits_fresh_usage() public {
        _install();
        _withdraw(AGENT, STORE, 50_000_000);
        bytes memory tight = vm.parseBytes(_trim(vm.readFile("test/fixtures/treasury_guard_tight.hex")));
        t.installPolicy(tight);
        _eq(t.x3Epoch(), 2, "second epoch");
        require(t.x3PolicyHash() == vm.parseBytes32(_trim(vm.readFile("test/fixtures/treasury_guard_tight.hash"))));
        // AGENT is no longer a caller (tight policy line 4); OPS is, but only 10_000 to BOB.
        _expectWithdrawRefused(AGENT, STORE, 1_000, 4, 4);
        _expectWithdrawRefused(OPS, BOB, 10_001, 6, 7);
        _expectWithdrawRefused(OPS, STORE, 10_000, 7, 9);
        vm.prank(OPS);
        t.withdraw(BOB, 10_000);
        vm.expectRevert(_refused(1, 5)); // setMemo no longer allowed: tight header line 1
        vm.prank(OPS);
        t.setMemo(bytes32("x"), false);
    }

    // ------------------------------------------------------------------ fuzz

    function testFuzz_first_withdraw_allowed_iff_within_per_call_bounds(uint256 amount) public {
        vm.assume(amount <= 10 ether);
        _install();
        vm.prank(AGENT);
        (bool ok, bytes memory ret) =
            address(t).call(abi.encodeWithSelector(X3ControlledTreasury.withdraw.selector, STORE, amount));
        if (amount > 50_000_000) {
            require(!ok && keccak256(ret) == keccak256(_refused(L_MAX, 7)), "above max refused");
        } else if (amount < 1_000) {
            require(!ok && keccak256(ret) == keccak256(_refused(L_MIN, 8)), "below min refused");
        } else {
            require(ok, "within bounds allowed");
        }
    }
}
