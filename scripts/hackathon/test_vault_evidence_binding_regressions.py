"""Adversarial evidence binding against the existing localhost-only RPC fixture.

These are synthetic observations. They neither sign transactions nor call a
public network, and a passing verifier test is not public-chain evidence.
"""
from __future__ import annotations

import base64
import copy
import json
from contextlib import contextmanager

import pytest

import test_vault_testnet_evidence as t

v = t.v


@contextmanager
def scenario(tmp_path):
    with t.Chain() as chain:
        # Chain.add_tx may retain action objects: never mutate RUN_STEPS globally.
        run, wasm = t.play(chain, tmp_path, steps=copy.deepcopy(t.RUN_STEPS))
        evidence = v.collect(run, wasm, chain.url)
        yield chain, run, wasm, evidence


def transaction(chain, run, label):
    steps = json.loads(run.read_text())["steps"]
    step = next(step for step in steps if step["label"] == label)
    return chain.txs[step["tx_hash"]]["transaction"]


def function_call(tx):
    calls = [action["FunctionCall"] for action in tx["actions"]
             if isinstance(action, dict) and "FunctionCall" in action]
    assert len(calls) == 1, "this scenario step must contain one function call"
    return calls[0]


def replace_args(call, args):
    call["args"] = base64.b64encode(json.dumps(args).encode()).decode()


def rehash(record):
    record["record_hash"] = v._canonical_hash(
        {key: value for key, value in record.items() if key != "record_hash"}
    )


def write_evidence(tmp_path, evidence):
    rehash(evidence)
    out = tmp_path / "rehashed-evidence.json"
    v._write_json(out, evidence)
    return out


@pytest.mark.parametrize("label", [
    "owner_funds", "owner_revokes", "owner_recovers", "owner_recovers_while_active",
])
@pytest.mark.parametrize("changed", ["method", "receiver", "deposit"])
def test_owner_step_requires_its_actual_method_receiver_and_deposit(tmp_path, label, changed):
    with scenario(tmp_path) as (chain, run, wasm, _):
        tx = transaction(chain, run, label)
        call = function_call(tx)
        if changed == "method":
            call["method_name"] = "unrelated_method"
        elif changed == "receiver":
            tx["receiver_id"] = t.STRANGER
        else:
            call["deposit"] = str(int(call["deposit"]) + 1)
        # Signer, transaction identity, success/refusal status and final state
        # remain unchanged: those facts cannot replace action binding.
        with pytest.raises(t.EvidenceError):
            v.collect(run, wasm, chain.url)


@pytest.mark.parametrize("label", ["owner_funds", "owner_revokes"])
def test_empty_argument_owner_calls_do_not_accept_substituted_arguments(tmp_path, label):
    with scenario(tmp_path) as (chain, run, wasm, _):
        replace_args(function_call(transaction(chain, run, label)), {"recipient": t.STRANGER})
        with pytest.raises(t.EvidenceError):
            v.collect(run, wasm, chain.url)


@pytest.mark.parametrize("label", ["owner_recovers", "owner_recovers_while_active"])
def test_recovery_amount_is_bound_to_the_scenario(tmp_path, label):
    with scenario(tmp_path) as (chain, run, wasm, _):
        call = function_call(transaction(chain, run, label))
        args = json.loads(base64.b64decode(call["args"]))
        args["amount"] = str(int(args["amount"]) + 1)
        replace_args(call, args)
        with pytest.raises(t.EvidenceError):
            v.collect(run, wasm, chain.url)


@pytest.mark.parametrize("field,value", [
    ("owner", t.STRANGER),
    ("agent", t.STRANGER),
    ("recipient", t.STRANGER),
    ("cap", "1"),
    ("budget", "1"),
    ("deadline_ms", 1),
])
def test_deployment_constructor_must_match_every_reviewed_term(tmp_path, field, value):
    with scenario(tmp_path) as (chain, run, wasm, _):
        call = function_call(transaction(chain, run, "vault_deployed"))
        args = json.loads(base64.b64decode(call["args"]))
        assert args[field] != value, "mutation must change a constructor field"
        args[field] = value
        replace_args(call, args)
        with pytest.raises(t.EvidenceError):
            v.collect(run, wasm, chain.url)


@pytest.mark.parametrize("label", ["vault_locked", "agent_key_scoped", "agent_full_key_deleted"])
def test_key_management_label_cannot_stand_in_for_its_action(tmp_path, label):
    with scenario(tmp_path) as (chain, run, wasm, _):
        tx = transaction(chain, run, label)
        tx["actions"] = [t.fn_action("unrelated_method", {})]
        # The final key list still looks correct, but the recorded operation is
        # no longer the claimed key change.
        with pytest.raises(t.EvidenceError):
            v.collect(run, wasm, chain.url)


def test_valid_manifest_for_another_account_is_not_vault_evidence(tmp_path):
    with scenario(tmp_path) as (chain, _, wasm, evidence):
        original_handle = chain.handle

        def other_account_also_has_code(method, params):
            result, error = original_handle(method, params)
            if (method == "query" and params.get("request_type") == "view_account"
                    and params.get("account_id") == t.OWNER and error is None):
                result = {**result, "code_hash": chain.code_hash}
            return result, error

        chain.handle = other_account_also_has_code
        changed = copy.deepcopy(evidence)
        changed["manifest"]["account_id"] = t.OWNER
        rehash(changed["manifest"])
        # This manifest is individually valid against the fake chain. Only
        # binding it to the run's vault reveals the substitution.
        v._verify_manifest_record(changed["manifest"], chain.url, wasm)
        out = write_evidence(tmp_path, changed)
        with pytest.raises(t.EvidenceError):
            v.verify(out, chain.url, wasm)


@pytest.mark.parametrize("operation", ["collect", "verify"])
def test_empty_code_sentinel_never_verifies_a_deployed_vault(tmp_path, operation):
    with scenario(tmp_path) as (chain, run, wasm, evidence):
        chain.code_hash = "1" * 32
        if operation == "collect":
            with pytest.raises(t.EvidenceError):
                v.collect(run, wasm, chain.url)
        else:
            changed = copy.deepcopy(evidence)
            changed["manifest"]["code_hash"] = chain.code_hash
            rehash(changed["manifest"])
            out = write_evidence(tmp_path, changed)
            with pytest.raises(t.EvidenceError):
                v.verify(out, chain.url, wasm)
