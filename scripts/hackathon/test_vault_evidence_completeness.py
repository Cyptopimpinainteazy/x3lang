"""Fail-closed evidence mutations against the complete fake-local-RPC scenario."""
import copy

import pytest

import test_vault_testnet_evidence as t

v = t.v


def write_rehashed(path, data):
    data["record_hash"] = v._canonical_hash({k: value for k, value in data.items() if k != "record_hash"})
    v._write_json(path, data)


@pytest.mark.parametrize("label", [row[0] for row in v.SCENARIO])
def test_every_operator_step_is_required(tmp_path, label):
    with t.Chain() as chain:
        run, wasm = t.play(chain, tmp_path)
        data = v._load_json(run)
        data["steps"] = [s for s in data["steps"] if s["label"] != label]
        v._write_json(run, data)
        with pytest.raises(t.EvidenceError, match="complete"):
            v.collect(run, wasm, chain.url)


@pytest.mark.parametrize("case", ["unlocked", "duplicate", "reorder", "unknown", "wrong_expectation", "wrong_signer", "duplicate_hash", "rpc_forged_success"])
def test_invalid_scenario_refuses_collection(tmp_path, case):
    with t.Chain() as chain:
        run, wasm = t.play(chain, tmp_path)
        data = v._load_json(run)
        if case == "unlocked":
            data["locked"] = False
            chain.keys[t.VAULT] = [{"public_key": t.FULL_PK, "access_key": {"nonce": 1, "permission": "FullAccess"}}]
        elif case == "duplicate":
            data["steps"].append(copy.deepcopy(data["steps"][0]))
        elif case == "reorder":
            data["steps"][0], data["steps"][1] = data["steps"][1], data["steps"][0]
        elif case == "unknown":
            data["steps"][0]["label"] = "unrelated_success"
        elif case == "wrong_expectation":
            data["steps"][10]["expect"] = "returns_true"
        elif case == "wrong_signer":
            data["steps"][0]["signer"] = t.AGENT
        elif case == "duplicate_hash":
            data["steps"][1]["tx_hash"] = data["steps"][0]["tx_hash"]
        else:
            next(s for s in data["steps"] if s["label"] == "agent_raw_transfer")["cli_exit"] = 0
        v._write_json(run, data)
        with pytest.raises(t.EvidenceError):
            v.collect(run, wasm, chain.url)


@pytest.mark.parametrize("case", ["all_missing", "one_missing", "duplicate", "unknown", "reorder", "wrong_hash", "verdict", "classification", "unlocked", "missing_run_step"])
def test_recomputed_public_hash_cannot_authorize_incomplete_evidence(tmp_path, case):
    with t.Chain() as chain:
        run, wasm = t.play(chain, tmp_path)
        data = v.collect(run, wasm, chain.url)
        out = tmp_path / "evidence.json"
        write_rehashed(out, data)
        assert v.verify(out, chain.url, wasm)["verified"] is True
        if case == "all_missing":
            data["transactions"] = []
        elif case == "one_missing":
            data["transactions"].pop()
        elif case == "duplicate":
            data["transactions"].append(copy.deepcopy(data["transactions"][0]))
        elif case == "unknown":
            data["transactions"][0]["label"] = "unrelated_success"
        elif case == "reorder":
            data["transactions"][0], data["transactions"][1] = data["transactions"][1], data["transactions"][0]
        elif case == "wrong_hash":
            record = data["transactions"][0]
            record["tx_hash"] = data["transactions"][1]["tx_hash"]
            record["transaction_hash"] = record["tx_hash"]
            record["record_hash"] = v._canonical_hash({k: x for k, x in record.items() if k != "record_hash"})
        elif case == "verdict":
            data["verdicts"][0]["verdict"] = "independently audited custody"
        elif case == "classification":
            data["classification"] = "PUBLIC_NEAR_TESTNET_CUSTODY_VAULT_NOT_MAINNET_NOT_AUDITED"
        elif case == "unlocked":
            data["run"]["locked"] = False
        else:
            data["run"]["steps"].pop()
        write_rehashed(out, data)
        with pytest.raises(t.EvidenceError):
            v.verify(out, chain.url, wasm)


def test_historical_before_balance_must_be_reread(tmp_path):
    with t.Chain() as chain:
        run, wasm = t.play(chain, tmp_path)
        data = v.collect(run, wasm, chain.url)
        chain.before_balances[t.VENDOR] = str(int(chain.before_balances[t.VENDOR]) + 1)
        out = tmp_path / "evidence.json"
        write_rehashed(out, data)
        with pytest.raises(t.EvidenceError, match="historical RPC"):
            v.verify(out, chain.url, wasm)


def test_before_snapshot_must_precede_payment_block(tmp_path):
    with t.Chain() as chain:
        run, wasm = t.play(chain, tmp_path)
        data = v._load_json(run)
        data["snapshots"]["before_attacks"].update(block_hash=t.FINAL["hash"], block_height=t.FINAL["height"])
        v._write_json(run, data)
        with pytest.raises(t.EvidenceError, match="precede"):
            v.collect(run, wasm, chain.url)


def test_scoped_key_identity_is_bound_to_real_payment_transaction(tmp_path):
    with t.Chain() as chain:
        run, wasm = t.play(chain, tmp_path)
        chain.txs[t.tx_hash(4)]["transaction"]["public_key"] = t.FULL_PK
        with pytest.raises(t.EvidenceError, match="scoped key"):
            v.collect(run, wasm, chain.url)


def test_local_cli_reports_never_become_independent_chain_proof(tmp_path):
    with t.Chain() as chain:
        run, wasm = t.play(chain, tmp_path)
        data = v.collect(run, wasm, chain.url)
        reports = [r for r in data["verdicts"] if r["tx_hash"] is None]
        assert len(reports) == 2
        assert all(r["evidence_level"] == "local_cli_report_not_independently_verified" for r in reports)
        out = tmp_path / "evidence.json"
        write_rehashed(out, data)
        result = v.verify(out, chain.url, wasm)
        assert result["local_cli_reports_independently_verified"] is False
        assert result["classification"] == v.CLASSIFICATION


@pytest.mark.parametrize("field,value", [("block_height", 999999), ("block_hash", "Z" * 44)])
def test_rehashed_final_block_must_match_rpc_header(tmp_path, field, value):
    with t.Chain() as chain:
        run, wasm = t.play(chain, tmp_path)
        data = v.collect(run, wasm, chain.url)
        data["final_block"][field] = value
        out = tmp_path / "evidence.json"
        write_rehashed(out, data)
        with pytest.raises(t.EvidenceError, match="final block differs"):
            v.verify(out, chain.url, wasm)


@pytest.mark.parametrize("kind", ["view_access_key_list", "call_function", "view_account"])
@pytest.mark.parametrize("field,value", [("block_height", 999999), ("block_hash", "Z" * 44)])
def test_final_queries_must_return_the_pinned_block(tmp_path, kind, field, value):
    with t.Chain() as chain:
        run, wasm = t.play(chain, tmp_path)
        data = v.collect(run, wasm, chain.url)
        original = chain.handle
        def wrong_block(method, params):
            result, error = original(method, params)
            if (method == "query" and params.get("request_type") == kind
                    and params.get("block_id") == t.FINAL["hash"]
                    and (kind != "view_account" or params.get("account_id") == t.VENDOR)):
                result = {**result, field: value}
            return result, error
        chain.handle = wrong_block
        out = tmp_path / "evidence.json"
        write_rehashed(out, data)
        with pytest.raises(t.EvidenceError, match="pinned"):
            v.verify(out, chain.url, wasm)


@pytest.mark.parametrize("value", [None, [], "not an object"])
@pytest.mark.parametrize("position", ["append", "interleave"])
@pytest.mark.parametrize("target", ["run_steps", "transaction_records"])
def test_invalid_list_entries_are_explicitly_refused(tmp_path, value, position, target):
    with t.Chain() as chain:
        run, wasm = t.play(chain, tmp_path)
        if target == "run_steps":
            data = v._load_json(run)
            rows = data["steps"]
        else:
            data = v.collect(run, wasm, chain.url)
            rows = data["transactions"]
        if position == "append":
            rows.append(value)
        else:
            rows[len(rows) // 2] = value
        if target == "run_steps":
            v._write_json(run, data)
            with pytest.raises(t.EvidenceError, match="list of objects"):
                v.collect(run, wasm, chain.url)
        else:
            out = tmp_path / "evidence.json"
            write_rehashed(out, data)
            with pytest.raises(t.EvidenceError, match="list of objects"):
                v.verify(out, chain.url, wasm)
