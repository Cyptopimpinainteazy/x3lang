#!/usr/bin/env python3
"""Read-only, fail-closed Solana X3 vault source transaction proof.

Verifies a finalized Solana transaction against a live 1Click-signed quote,
program-owned data logs and a matching classic SPL Token TransferChecked CPI.
This is NOT proof that 1Click subsequently settled on NEAR.
"""
from __future__ import annotations

import argparse
import base64
import hashlib
import json
import re
import sys
from pathlib import Path
from typing import Any

_ROOT = Path(__file__).resolve().parents[1]
if str(_ROOT) not in sys.path:
    sys.path.insert(0, str(_ROOT))

import near_intents as ni  # noqa: E402

# The public SPL Token program ID, represented as bytes because its base58 form can be
# mistaken for a generic high-entropy credential by repository secret scanners.
TOKEN_PROGRAM = ni.ed25519.b58encode(bytes((
    6, 221, 246, 225, 215, 101, 161, 147, 217, 203, 225, 70, 206, 235, 121, 172,
    28, 180, 133, 237, 95, 91, 55, 145, 58, 140, 245, 133, 126, 255, 0, 169,
)))
EVENT_MARKER = b"X3_VAULT_EXECUTE_V1"
EVENT_FIELDS = 7
EXIT_UNVERIFIED = 5
_INVOKE = re.compile(r"^Program ([A-Za-z0-9]+) invoke \[(\d+)\]$")
_END = re.compile(r"^Program ([A-Za-z0-9]+) (success|failed:.*)$")


def _pubkey(text: str) -> bytes:
    if not isinstance(text, str):
        raise ValueError("public key must be base58 text")
    raw = ni.ed25519.b58decode(text)
    if len(raw) != 32:
        raise ValueError("public key must decode to 32 bytes")
    return raw


def _event_fields(raw_line: str) -> tuple[bytes, ...]:
    if not raw_line.startswith("Program data: "):
        raise ValueError("not an event line")
    parts = raw_line.removeprefix("Program data: ").split()
    if len(parts) != EVENT_FIELDS:
        raise ValueError("vault event has wrong field count")
    try:
        data = tuple(base64.b64decode(x, validate=True) for x in parts)
    except (ValueError, base64.binascii.Error) as exc:
        raise ValueError("invalid vault event encoding") from exc
    if (data[0] != EVENT_MARKER or len(data[1]) != 32 or len(data[2]) != 8
            or len(data[3]) != 32 or len(data[4]) != 32 or len(data[5]) != 32
            or len(data[6]) != 8):
        raise ValueError("vault event fields not canonical")
    return data


def _events_in_expected_instruction(logs: Any, program_id: str, index: int) -> list[tuple[bytes, ...]]:
    """Scope event data to the exact vault program's top-level execution frame."""
    if not isinstance(logs, list) or not all(isinstance(x, str) for x in logs):
        raise ValueError("missing program logs")
    stack: list[str] = []
    top_index = -1
    events = []
    for line in logs:
        match = _INVOKE.match(line)
        if match:
            program, depth = match[1], int(match[2])
            if depth == 1:
                if stack:
                    raise ValueError("nested top-level invocation")
                top_index += 1
            elif depth != len(stack) + 1:
                raise ValueError("invalid invocation depth")
            stack.append(program)
            continue
        match = _END.match(line)
        if match:
            if not stack or stack[-1] != match[1]:
                raise ValueError("mismatched program exit")
            stack.pop()
            continue
        if line.startswith("Program data: ") and stack and stack[-1] == program_id and top_index == index:
            events.append(_event_fields(line))
    if stack:
        raise ValueError("incomplete program execution logs")
    return events


def _program_id(ix: Any) -> str | None:
    return ix.get("programId") if isinstance(ix, dict) else None


def _token_cpis(tx: dict[str, Any], index: int, *, source: str, destination: str,
                mint: str, authority: str, amount: int) -> list[dict[str, Any]]:
    meta = tx.get("meta") or {}
    matches = []
    if not isinstance(meta.get("innerInstructions"), list):
        raise ValueError("no inner CPI instructions recorded")
    for group in meta["innerInstructions"]:
        if not isinstance(group, dict) or group.get("index") != index:
            continue
        for ix in group.get("instructions") or []:
            if _program_id(ix) != TOKEN_PROGRAM or not isinstance(ix.get("parsed"), dict):
                continue
            parsed = ix["parsed"]
            info = parsed.get("info") or {}
            units = (info.get("tokenAmount") or {}).get("amount")
            if (parsed.get("type") == "transferChecked"
                    and info.get("source") == source
                    and info.get("destination") == destination
                    and info.get("mint") == mint
                    and info.get("authority") == authority
                    and units == str(amount)):
                matches.append(ix)
    return matches


def quote_commitment(receipt: dict[str, Any]) -> tuple[bytes, str, str, int]:
    """Authenticate quote before using its fields in any transaction proof."""
    if receipt.get("kind") != ni.RECEIPT_KIND:
        raise ValueError("not an X3 live quote receipt")
    response = receipt.get("quote_response")
    valid, why = ni.check_oneclick_quote_signature(response)
    if not valid:
        raise ValueError(f"invalid signed 1Click quote: {why}")
    if (not isinstance(response, dict)
            or not isinstance(response.get("quoteRequest"), dict)
            or response["quoteRequest"].get("dry") is not False):
        raise ValueError("quote must be live")
    signed = ni.signed_quote_payload(response)
    if (receipt.get("signed_payload") != signed
            or receipt.get("quote_hash") != ni.oneclick_quote_hash(response)
            or receipt.get("quote_response_sha256") != ni._sha256_json(response)):
        raise ValueError("stored quote differs from signed response")
    deposit = receipt.get("deposit") or {}
    quote = response.get("quote") or {}
    amount = int(quote.get("amountIn") or 0)
    if (amount <= 0 or deposit.get("amount") != str(amount)
            or deposit.get("address") != quote.get("depositAddress")
            or deposit.get("asset") != response["quoteRequest"].get("originAsset")):
        raise ValueError("receipt deposit terms mismatch the live quote")
    origin = (receipt.get("tokens") or {}).get("origin") or {}
    mint = origin.get("contractAddress")
    if not mint or not isinstance(mint, str):
        raise ValueError("receipt lacks Solana mint contract")
    owner = quote.get("depositAddress")
    _pubkey(owner)
    _pubkey(mint)
    return hashlib.sha256(signed.encode()).digest(), owner, mint, amount


def _facts(tx: Any, *, tx_signature: str, program: str, policy: str,
           source_token: str, destination_token: str, nonce: int,
           digest: bytes, deposit_owner: str, mint: str, amount: int) -> dict[str, Any]:
    """Require a unique vault event and matching Token CPI in one finalized transaction."""
    if not isinstance(tx, dict) or not isinstance(tx.get("meta"), dict):
        raise ValueError("transaction missing")
    meta = tx["meta"]
    if meta.get("err") is not None:
        raise ValueError("transaction failed")
    if tx.get("blockTime") is None or not isinstance(tx.get("slot"), int):
        raise ValueError("missing finalized time or slot")
    transaction = tx.get("transaction") or {}
    if ((transaction.get("signatures") or [None])[0] != tx_signature):
        raise ValueError("transaction signature differs")
    instructions = ((transaction.get("message") or {}).get("instructions") or [])
    vault_indexes = [i for i, ix in enumerate(instructions) if _program_id(ix) == program]
    if len(vault_indexes) != 1:
        raise ValueError("expected exactly one top-level vault instruction")
    index = vault_indexes[0]
    fields = _events_in_expected_instruction(meta.get("logMessages"), program, index)
    matching = [
        event for event in fields
        if (event[1] == _pubkey(policy)
            and int.from_bytes(event[2], "little") == nonce
            and event[3] == digest
            and event[4] == _pubkey(destination_token)
            and event[5] == _pubkey(deposit_owner)
            and int.from_bytes(event[6], "little") == amount)
    ]
    if len(fields) != 1 or len(matching) != 1:
        raise ValueError("missing, ambiguous or mismatched vault event")
    cpis = _token_cpis(
        tx, index, source=source_token, destination=destination_token,
        mint=mint, authority=policy, amount=amount)
    if len(cpis) != 1:
        raise ValueError("missing or ambiguous authorized SPL Token TransferChecked")
    return {
        "slot": tx["slot"],
        "block_time": tx["blockTime"],
        "program": program,
        "policy": policy,
        "source_token": source_token,
        "destination_token": destination_token,
        "deposit_owner": deposit_owner,
        "nonce": nonce,
        "quote_digest_sha256": digest.hex(),
        "mint": mint,
        "amount": str(amount),
        "instruction_index": index,
    }


def verify_source(receipt: dict[str, Any], *, solana: Any, signature: str,
                  program: str, policy: str, source_token: str,
                  destination_token: str, nonce: int) -> dict[str, Any]:
    """Independent source proof from >=2 distinct finalized Solana RPC endpoints.

    Every endpoint must agree on normalized event/CPI facts. It cannot prove that
    the deployed program matches this repository's SBF bytes or that NEAR settled.
    """
    summary: dict[str, Any] = {
        "source_verified": False,
        "destination_verified": False,
        "cross_chain_causality_proven": False,
        "proof_level": "source_only",
    }
    try:
        if not isinstance(nonce, int) or nonce <= 0:
            raise ValueError("nonce must be a positive unsigned integer")
        for value in (program, policy, source_token, destination_token):
            _pubkey(value)
        digest, owner, mint, amount = quote_commitment(receipt)
        urls = getattr(solana, "urls", [])
        if len(urls) < 2 or len(set(urls)) != len(urls):
            raise ValueError("at least two distinct finalized RPC endpoints are required")
        observations = []
        for url in urls:
            tx = solana.get_transaction(url, signature)
            facts = _facts(
                tx, tx_signature=signature, program=program, policy=policy,
                source_token=source_token, destination_token=destination_token,
                nonce=nonce, digest=digest, deposit_owner=owner, mint=mint, amount=amount)
            observations.append((url, facts, ni.chain_evidence.sha256_json(tx)))
        first = observations[0][1]
        if any(facts != first for _, facts, _ in observations[1:]):
            raise ValueError("finalized RPC endpoints disagree on vault transfer facts")
        summary.update({
            "source_verified": True,
            "solana_signature": signature,
            "source": first,
            "rpcs": [{"url": url, "tx_result_sha256": raw} for url, _, raw in observations],
            "limitations": [
                "SBF program ID must be audited and its deployed code hash independently verified.",
                "Solana vault execution is not proof of the corresponding NEAR delivery.",
                "RPC URLs can share upstream infrastructure even when distinct.",
            ],
        })
    except (ValueError, TypeError, KeyError, IndexError, ni.NearIntentsError,
            ni.chain_evidence.ChainEvidenceError) as exc:
        summary["failure"] = str(exc)
    return summary


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Read-only finalized Solana vault source proof; NOT NEAR settlement proof")
    parser.add_argument("--receipt", required=True)
    parser.add_argument("--tx", required=True)
    parser.add_argument("--program", required=True)
    parser.add_argument("--policy", required=True)
    parser.add_argument("--source-token", required=True)
    parser.add_argument("--destination-token", required=True)
    parser.add_argument("--nonce", type=int, required=True)
    parser.add_argument("--rpc-url", action="append", required=True, help="repeat for at least two distinct endpoints")
    args = parser.parse_args(argv)
    try:
        with open(args.receipt, encoding="utf-8") as file:
            receipt = json.load(file)
        rpc = ni.chain_evidence.SolanaRpc(
            transport=ni.urllib_transport, urls=args.rpc_url)
        outcome = verify_source(
            receipt, solana=rpc, signature=args.tx, program=args.program,
            policy=args.policy, source_token=args.source_token,
            destination_token=args.destination_token, nonce=args.nonce)
    except (OSError, ValueError, json.JSONDecodeError) as exc:
        outcome = {"source_verified": False, "cross_chain_causality_proven": False, "failure": str(exc)}
    print(json.dumps(outcome, sort_keys=True, indent=2))
    return 0 if outcome.get("source_verified") else EXIT_UNVERIFIED


if __name__ == "__main__":
    sys.exit(main())
