"""The local forcefield: twenty checks, each with a mutation that must fail."""

import hashlib
import json
import shutil
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "x3-lang"))

import forcefield  # noqa: E402


PAYMENT_SHA = "9ee29a5603cfa1e7d9fdbd866cf330ce35e6b5f0907c6844dd2ef1a213768ba0"
MANDATE_SHA = "eea021d5cc1118801f4e1d3b9c0efe89fc59a7f654031f9ff1951aa1f2f3d515"
QUOTE = "8ynPgyqDugkyL2gicfUuTtymMjTPQzcRMBjj4RKgKeSD"


def test_refusal_certificate_binds_program_guard_and_quote():
    first = forcefield.refusal_certificate(
        program_sha256=PAYMENT_SHA, mandate_sha256=MANDATE_SHA, guard="mandate_recipient", quote_hash=QUOTE,
    )
    second = forcefield.refusal_certificate(
        program_sha256=PAYMENT_SHA, mandate_sha256=MANDATE_SHA, guard="mandate_recipient", quote_hash=QUOTE,
    )
    other = forcefield.refusal_certificate(
        program_sha256=PAYMENT_SHA, mandate_sha256=MANDATE_SHA, guard="mandate_refund_to", quote_hash=QUOTE,
    )
    assert first == second
    assert first["certificate_sha256"] != other["certificate_sha256"]
    assert first["certificate_sha256"] == hashlib.sha256(first["canonical"].encode()).hexdigest()


def test_repo_source_has_no_bypass_method():
    found = forcefield.scan_contract_source(
        forcefield.GUARD_LIB.read_text(encoding="utf-8"),
        forcefield.GUARD_MANDATE.read_text(encoding="utf-8"),
    )
    assert found["ok"] is True
    assert found["bypass_present"] == []
    planted = forcefield.scan_contract_source("fn owner_execute", forcefield.GUARD_MANDATE.read_text(encoding="utf-8"))
    assert planted["ok"] is False
    assert "fn owner_execute" in planted["bypass_present"]
    assert forcefield._has_token_withdraw("pub fn withdraw (self)") is True
    assert forcefield._has_token_withdraw("pub fn withdraw_agent_storage(&mut self, agent: AccountId)") is False


def test_outcome_authentication_rejects_swapped_hashes():
    reported = {
        "status": "Settled",
        "amount_out": "24995200",
        "tx_hashes": ["solana-sig", "near-tx"],
    }
    solana = {
        "signature": "solana-sig",
        "amount": 25_000_000,
        "deposit_address": "deposit",
        "deposit_address_delta": 25_000_000,
        "funder_delta": -25_000_000,
    }
    near = {"tx_hash": "near-tx", "recipient": "x3lang.near", "amount": 24_995_200, "logs_ok": True}
    live = forcefield.authenticate_outcome(
        reported=reported, solana=solana, near=near, floors=[24_770_243, 24_750_000], quote_deposit_address="deposit",
    )
    swapped = dict(reported)
    swapped["tx_hashes"] = ["near-tx", "solana-sig"]
    refused = forcefield.authenticate_outcome(
        reported=swapped, solana=solana, near=near, floors=[24_770_243, 24_750_000], quote_deposit_address="deposit",
    )
    narrated = dict(reported)
    narrated["amount_out"] = "1"
    short = forcefield.authenticate_outcome(
        reported=narrated, solana=solana, near=near, floors=[24_770_243, 24_750_000], quote_deposit_address="deposit",
    )
    assert live["ok"] is True and live["cross_chain_proof"] is False
    assert refused["ok"] is False and "tx_hashes" in refused["reasons"]
    assert short["ok"] is False


def test_live_keys_stay_open_and_a_separated_fixture_passes():
    live = json.loads(forcefield.OBSERVATION.read_text(encoding="utf-8"))["agent_access_keys"]["result"]["keys"]
    classified = forcefield.classify_keys(live)
    assert classified["separated"] is False
    assert classified["status"] == "action_required"
    assert classified["full_access"] == ["ed25519:8teaXGYFELKQN5QA7gQqeA4B7URrcMQqaczZ5D68c23u"]
    only = [key for key in live if key["public_key"].startswith("ed25519:J3Ni")]
    separated = forcefield.classify_keys(only)
    assert separated["separated"] is True
    assert separated["status"] == "pass"
    assert separated["open"] == []


def test_unfunded_accept_releases_the_budget_and_keeps_the_quote():
    held = forcefield.budget_after_accept(
        settled=False, funded=False, accepted_at_ms=1000, now_ms=1999, window_ms=1000, quote_consumed=True,
    )
    released = forcefield.budget_after_accept(
        settled=False, funded=False, accepted_at_ms=1000, now_ms=2000, window_ms=1000, quote_consumed=True,
    )
    assert held["state"] == "reserved"
    assert released["state"] == "released" and released["quote_consumed"] is True


def test_verifier_key_constant_has_no_setter():
    found = forcefield.verifier_key_lock(forcefield.GUARD_LIB.read_text(encoding="utf-8"))
    assert found["ok"] is True
    assert found["setter_present"] is False
    planted = forcefield.verifier_key_lock(forcefield.GUARD_LIB.read_text(encoding="utf-8") + "\nfn set_manager")
    assert planted["ok"] is False


def test_unenforceable_edits_of_the_invoice_fail_to_compile():
    source = forcefield.PAYMENT.read_text(encoding="utf-8")
    found = forcefield.compiler_refusals(source)
    assert found["baseline_compiles"] is True
    assert found["attacker_compiles"] is True
    assert found["ok"] is True
    codes = {row["case"]: row["code"] for row in found["cases"]}
    assert codes["finality_promise"] == "X3_NEAR_TYPECHECK"
    assert codes["no_slippage_bound"] == "X3_NEAR_SLIPPAGE_REQUIRED"
    assert codes["second_leg"] == "X3_NEAR_ROUTE_SHAPE"
    assert codes["lookalike_recipient"] == "X3_NEAR_TYPECHECK"


def test_mandate_diff_names_the_changed_clause():
    source = forcefield.MANDATE.read_text(encoding="utf-8")
    same = forcefield.mandate_diff(source, source)
    changed = forcefield.mandate_diff(source, source.replace("per_24h 25000000", "per_24h 25000001", 1))
    assert same["changed"] == []
    assert changed["changed"] == ["per_24h"]


def test_extensions_strip_before_the_official_compiler_and_stay_off_chain_terms():
    source = forcefield.MANDATE.read_text(encoding="utf-8")
    extended = source.replace(
        "    allow recipients x3lang.near\n",
        "    allow recipients x3lang.near\n"
        "    deny recipients attacker.near\n"
        f"    allow programs {PAYMENT_SHA}\n"
        "    agents shopper.x3lang.near\n",
        1,
    )
    tokens = json.loads(forcefield.TOKENS.read_text(encoding="utf-8"))
    now = datetime(2026, 11, 12, 1, 49, 21, tzinfo=timezone.utc)
    original = forcefield.compile_mandate_source(source, tokens, now=now)
    compiled = forcefield.compile_mandate_source(extended, tokens, now=now)
    assert compiled["stripped"] == source
    assert compiled["extras"]["deny_recipients"] == ["attacker.near"]
    assert compiled["extras"]["allow_programs"] == [PAYMENT_SHA]
    assert compiled["extras"]["agents"] == ["shopper.x3lang.near"]
    assert compiled["chain_terms"] == original["chain_terms"]
    assert "deny_recipients" not in compiled["chain_terms"]
    try:
        forcefield.split_forcefield("    allow programs not-a-hash\n")
    except forcefield.ForcefieldError as exc:
        assert exc.code == "X3_FORCEFIELD_PROGRAM"
    else:
        raise AssertionError("a short program pin compiled")


def test_signature_bit_does_not_authorize_a_broken_recipient_or_refund():
    report = _report()
    row = _by_name(report, "signature_is_not_permission")
    refund = _by_name(report, "refund_wallet_pinned")
    assert row["status"] == "pass"
    assert row["detail"]["signature_reverified"] is False
    assert row["detail"]["attacker_failed"] == ["mandate_recipient"]
    assert refund["status"] == "pass"
    assert "mandate_refund_to" in refund["detail"]["failed"]


def test_approval_records_the_program_and_refuses_a_wider_mandate():
    terms = {"recipients": ["x3lang.near"], "per_payment_cap": "25000000", "refund_to": "wallet"}
    refused = forcefield.apply_approval(
        terms, approver="x3lang.near", program_sha256=PAYMENT_SHA,
        widen={"recipient": "attacker.near", "per_payment_cap": "25000001"},
    )
    signed = forcefield.apply_approval(terms, approver="x3lang.near", program_sha256=PAYMENT_SHA)
    assert refused["ok"] is False
    assert terms == {"recipients": ["x3lang.near"], "per_payment_cap": "25000000", "refund_to": "wallet"}
    assert signed["ok"] is True
    assert signed["approvals"] == [{"approver": "x3lang.near", "program_sha256": PAYMENT_SHA}]


def test_bad_signature_and_rejected_policy_do_not_burn_a_quote():
    bad = forcefield.consider_quote(
        forcefield.empty_ledger(), signature_ok=False, policy_ok=True,
        quote_hash=QUOTE, deposit_address="deposit", intent_key="intent",
    )
    rejected = forcefield.consider_quote(
        forcefield.empty_ledger(), signature_ok=True, policy_ok=False,
        quote_hash=QUOTE, deposit_address="deposit", intent_key="intent",
    )
    accepted = forcefield.consider_quote(
        forcefield.empty_ledger(), signature_ok=True, policy_ok=True,
        quote_hash=QUOTE, deposit_address="deposit", intent_key="intent",
    )
    replay = forcefield.consider_quote(
        accepted["ledger"], signature_ok=True, policy_ok=True,
        quote_hash=QUOTE, deposit_address="other-deposit", intent_key="again",
    )
    assert bad["consumed"] is False and bad["reason"] == "bad_signature"
    assert rejected["consumed"] is False and rejected["reason"] == "rejected_policy"
    assert accepted["consumed"] is True
    assert replay["consumed"] is False and replay["reason"] == "replay"


def test_agent_methods_take_no_purpose_and_invoice_calls_attach_zero():
    source = forcefield.GUARD_MANDATE.read_text(encoding="utf-8")
    for name in ("agent_accept_quote", "agent_record_outcome"):
        signature = forcefield.function_signature(source, name)
        assert "decision_hash" not in signature
        assert "purpose" not in signature
    report = _report()
    assert _by_name(report, "no_purpose_string")["status"] == "pass"


def test_denylist_beats_the_allowlist():
    both = forcefield.recipient_decision("x3lang.near", deny=("x3lang.near",), allow=("x3lang.near",))
    allowed = forcefield.recipient_decision("x3lang.near", deny=("attacker.near",), allow=("x3lang.near",))
    assert both["ok"] is False and both["guard"] == "mandate_deny_recipient"
    assert allowed["ok"] is True


def test_program_pin_is_one_use_and_absent_pins_keep_current_behavior():
    assert forcefield.program_allowed("ab" * 32, (), {})["ok"] is True
    assert forcefield.requires_new_version((), "cd" * 32) is False
    first = forcefield.program_allowed(PAYMENT_SHA, (PAYMENT_SHA,), {})
    second = forcefield.program_allowed(PAYMENT_SHA, (PAYMENT_SHA,), {PAYMENT_SHA: 1})
    other = "a" * 64
    assert first["ok"] is True
    assert second["guard"] == "mandate_program_reuse"
    assert forcefield.program_allowed(other, (PAYMENT_SHA,), {})["guard"] == "mandate_program"
    assert forcefield.requires_new_version((PAYMENT_SHA,), other) is True


def test_hash_chain_commits_to_the_previous_version():
    terms = {"per_payment_cap": "25000000"}
    first = forcefield.link(
        previous_hash=None, version=1, mandate_sha256=MANDATE_SHA, program_sha256=PAYMENT_SHA, terms=terms,
    )
    second = forcefield.link(
        previous_hash=first["chain_hash"], version=2, mandate_sha256=MANDATE_SHA,
        program_sha256=PAYMENT_SHA, terms=terms,
    )
    assert first["previous_hash"] == forcefield.GENESIS_HASH
    assert forcefield.verify_chain([first, second]) is True
    broken = dict(second)
    broken["terms"] = {"per_payment_cap": "1"}
    assert forcefield.verify_chain([first, broken]) is False


def test_proof_page_embeds_the_files_and_javascript_matches_sha256():
    payment = forcefield.PAYMENT.read_bytes()
    mandate = forcefield.MANDATE.read_bytes()
    page = forcefield.PROOF_PAGE.read_text(encoding="utf-8")
    assert page == forcefield.proof_page(payment.decode(), mandate.decode())
    assert forcefield.extract_source(page, "payment-source").encode() == payment
    assert forcefield.extract_source(page, "mandate-source").encode() == mandate
    node = shutil.which("node")
    assert node
    script = """
const {sha256Hex} = require(process.argv[1]);
const fs = require("fs");
const page = fs.readFileSync(process.argv[2], "utf8");
function extract(id) {
  const start = '<script type="text/plain" id="' + id + '">';
  const begin = page.indexOf(start) + start.length;
  return page.slice(begin, page.indexOf("</script>", begin));
}
for (const id of ["payment-source", "mandate-source"]) {
  process.stdout.write(sha256Hex(extract(id)) + "\\n");
}
process.stdout.write(sha256Hex("") + "\\n");
process.stdout.write(sha256Hex("abc") + "\\n");
"""
    result = subprocess.run(
        [node, "-e", script, str(forcefield.SHA256_JS), str(forcefield.PROOF_PAGE)],
        check=True, capture_output=True, text=True,
    )
    payment_got, mandate_got, empty, abc = result.stdout.splitlines()
    assert payment_got == hashlib.sha256(payment).hexdigest() == PAYMENT_SHA
    assert mandate_got == hashlib.sha256(mandate).hexdigest() == MANDATE_SHA
    assert empty == "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    assert abc == "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"


def test_shared_ceiling_counts_every_named_agent():
    spends = {
        "a.near": [{"at_ms": 10_000, "amount": "40"}],
        "b.near": [{"at_ms": 10_000, "amount": "50"}],
    }
    spent, count = forcefield.shared_spent(spends, 10_001, ("a.near", "b.near"), window_ms=86_400_000)
    assert spent == 90 and count == 2
    assert forcefield.shared_ceiling_ok(spent, count, 11, 100) is False
    assert forcefield.shared_ceiling_ok(spent, count, 10, 100) is True
    aged, aged_count = forcefield.shared_spent(
        {"a.near": [{"at_ms": 0, "amount": "100"}]}, 86_400_000, ("a.near",), window_ms=86_400_000,
    )
    assert aged == 0 and aged_count == 0


def test_attempt_cap_stores_eight_certificates_and_refuses_the_ninth():
    log = []
    last = {"log": log, "ok": True}
    for _ in range(forcefield.ATTEMPT_CAP):
        last = forcefield.record_attempt(
            last["log"], program_sha256=PAYMENT_SHA, mandate_sha256=MANDATE_SHA,
            guard="mandate_recipient", quote_hash=QUOTE,
        )
    overflow = forcefield.record_attempt(
        last["log"], program_sha256=PAYMENT_SHA, mandate_sha256=MANDATE_SHA,
        guard="mandate_recipient", quote_hash=QUOTE,
    )
    assert last["used"] == 8
    assert last["log"][0]["guard"] == "mandate_recipient"
    assert overflow["ok"] is False
    assert overflow["guard"] == "attempt_budget"
    assert len(overflow["log"]) == 8


def test_vault_transfer_requires_the_mandate_and_refuses_withdraw():
    assert forcefield.vault_may_transfer(
        guards_ok=True, method="vault_spend", predecessor_is_agent=True, withdraw=False,
    )["ok"] is True
    assert forcefield.vault_may_transfer(
        guards_ok=True, method="withdraw", predecessor_is_agent=True, withdraw=True,
    )["ok"] is False
    assert forcefield.vault_may_transfer(
        guards_ok=False, method="vault_spend", predecessor_is_agent=True, withdraw=False,
    )["ok"] is False
    assert forcefield.vault_may_transfer(
        guards_ok=True, method="vault_spend", predecessor_is_agent=False, withdraw=False,
    )["ok"] is False


def test_evaluate_passes_with_the_live_key_left_open():
    report = _report()
    assert report["ok"] is True
    assert [row["name"] for row in report["rows"]] == list(forcefield.CHECKS)
    assert all(row["status"] == "pass" for row in report["rows"] if row["name"] != "one_key")
    key = _by_name(report, "one_key")
    assert key["status"] == "open"
    assert report["open"]
    assert "FullAccess" in key["summary"]
    rendered = forcefield.render(report)
    assert "OPEN" in rendered and "ok: true" in rendered


_CACHED = None


def _report():
    global _CACHED
    if _CACHED is None:
        _CACHED = forcefield.evaluate()
    return _CACHED


def _by_name(report, name):
    return next(row for row in report["rows"] if row["name"] == name)
