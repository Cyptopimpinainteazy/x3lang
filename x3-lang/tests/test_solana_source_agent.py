"""Signing-boundary tests; public-chain completion is tested separately."""
import importlib.util
from pathlib import Path

import pytest

PATH = Path(__file__).parents[1] / "solana" / "source_agent.py"


def load():
    spec = importlib.util.spec_from_file_location("solana_source_agent", PATH)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def test_execute_instruction_is_agent_only_and_exact_abi():
    from solders.keypair import Keypair
    from solders.pubkey import Pubkey

    m = load()
    program, owner, agent, source, dest, mint = [Keypair().pubkey() for _ in range(6)]
    policy, _ = Pubkey.find_program_address([b"x3-vault", bytes(owner), bytes(source)], program)
    ix = m.execute_instruction(program, agent, policy, source, dest, mint, 1, bytes(range(32)))
    assert ix.data == b"\x02" + (1).to_bytes(8, "little") + bytes(range(32))
    assert [a.pubkey for a in ix.accounts] == [agent, policy, source, dest, mint, m.TOKEN_PROGRAM]
    assert [a.pubkey for a in ix.accounts if a.is_signer] == [agent]
    assert owner not in [a.pubkey for a in ix.accounts]


def test_policy_decoder_rejects_truncated_and_trailing_data():
    m = load()
    for raw in [b"", bytes(1394), bytes(1396)]:
        with pytest.raises(m.SourceAgentError):
            m.decode_policy(raw)


def test_signer_refuses_plan_digest_mismatch_without_reading_key(tmp_path):
    m = load()
    with pytest.raises(m.SourceAgentError, match="PLAN_DIGEST"):
        m.sign_transaction({"schema": "untrusted"}, "0" * 64, tmp_path / "absent-key")
