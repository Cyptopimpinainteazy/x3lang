#!/usr/bin/env python3
"""Generate X3 Ethereum bridge proof fixture JSON from RPC plus archived trie nodes.

Standard Ethereum JSON-RPC exposes transaction receipts and block headers, but
not receipt trie inclusion nodes. This tool fetches the canonical receipt/header
fields, imports `receipt_key` and `trie_nodes` from an archive JSON produced by a
proof service or node plugin, and emits the JSON schema consumed by
`EthereumLightClientVerifier`.
"""

from __future__ import annotations

import argparse
import json
import sys
import urllib.request
from typing import Any

ERC20_TRANSFER_TOPIC = (
    "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef"
)


def rpc(url: str, method: str, params: list[Any]) -> Any:
    body = json.dumps(
        {"jsonrpc": "2.0", "id": 1, "method": method, "params": params}
    ).encode("utf-8")
    request = urllib.request.Request(
        url,
        data=body,
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    with urllib.request.urlopen(request, timeout=20) as response:
        payload = json.loads(response.read().decode("utf-8"))
    if "error" in payload:
        raise SystemExit(f"{method} RPC error: {payload['error']}")
    if "result" not in payload:
        raise SystemExit(f"{method} RPC response missing result")
    return payload["result"]


def normalize_hex(value: str) -> str:
    if not isinstance(value, str):
        raise TypeError(f"expected hex string, got {type(value).__name__}")
    value = value.lower()
    return value if value.startswith("0x") else f"0x{value}"


def require_hex(value: str, field: str, bytes_len: int | None = None) -> str:
    value = normalize_hex(value)
    raw = value[2:]
    if len(raw) % 2 != 0:
        raise ValueError(f"{field} must have even hex length")
    int(raw or "0", 16)
    if bytes_len is not None and len(raw) != bytes_len * 2:
        raise ValueError(f"{field} must be {bytes_len} bytes")
    return value


def indexed_address_topic(address: str) -> str:
    address = normalize_hex(address)[2:]
    if len(address) != 40:
        raise ValueError("receiver address must be 20 bytes for ERC-20 topic")
    return "0x" + ("0" * 24) + address


def evm_word(value: int) -> str:
    if value < 0:
        raise ValueError("amount must be non-negative")
    return f"0x{value:064x}"


def load_archive(path: str) -> dict[str, Any]:
    with open(path, "r", encoding="utf-8") as handle:
        archive = json.load(handle)
    if not isinstance(archive, dict):
        raise ValueError("archive must be a JSON object")
    return archive


def archive_header_hash(archive: dict[str, Any]) -> str:
    value = archive.get("header_hash") or archive.get("source", {}).get("block_hash")
    if not value:
        raise ValueError("archive must contain header_hash or source.block_hash")
    return require_hex(value, "archive.header_hash", 32)


def archive_receipts_root(archive: dict[str, Any]) -> str:
    value = archive.get("receipts_root")
    if not value:
        raise ValueError("archive must contain receipts_root")
    return require_hex(value, "archive.receipts_root", 32)


def extract_transfer_log(
    receipt: dict[str, Any], token_address: str, receiver: str, amount: int
) -> dict[str, Any]:
    token_address = normalize_hex(token_address)
    receiver_topic = indexed_address_topic(receiver)
    amount_word = evm_word(amount)
    for log in receipt.get("logs", []):
        topics = [normalize_hex(topic) for topic in log.get("topics", [])]
        if (
            normalize_hex(log.get("address", "")) == token_address
            and len(topics) >= 3
            and topics[0] == ERC20_TRANSFER_TOPIC
            and topics[2] == receiver_topic
            and normalize_hex(log.get("data", "")) == amount_word
        ):
            return {
                "address": normalize_hex(log["address"]),
                "topics": topics,
                "data": normalize_hex(log["data"]),
            }
    raise ValueError("receipt does not contain matching ERC-20 Transfer log")


def validate_finality_proof(proof: dict[str, Any]) -> None:
    if proof.get("proof_type") != "ethereum-header-rlp-v1":
        raise ValueError("source_finality_proof.proof_type mismatch")
    require_hex(proof["rlp_header"], "source_finality_proof.rlp_header")
    require_hex(proof["header_hash"], "source_finality_proof.header_hash", 32)
    require_hex(proof["receipts_root"], "source_finality_proof.receipts_root", 32)


def validate_transfer_proof(proof: dict[str, Any]) -> None:
    if proof.get("proof_type") != "ethereum-receipt-trie-v1":
        raise ValueError("transfer_proof.proof_type mismatch")
    require_hex(proof["receipt_key"], "transfer_proof.receipt_key")
    require_hex(proof["receipt_rlp"], "transfer_proof.receipt_rlp")
    require_hex(proof["receipt_hash"], "transfer_proof.receipt_hash", 32)
    require_hex(proof["receipts_root"], "transfer_proof.receipts_root", 32)
    nodes = proof.get("trie_nodes")
    if not isinstance(nodes, list) or not nodes:
        raise ValueError("transfer_proof.trie_nodes must be a non-empty list")
    for idx, node in enumerate(nodes):
        require_hex(node, f"transfer_proof.trie_nodes[{idx}]")
    if "log" in proof:
        log = proof.get("log")
        if not isinstance(log, dict):
            raise ValueError("transfer_proof.log must be an object")
        require_hex(log["address"], "transfer_proof.log.address", 20)
        require_hex(log["data"], "transfer_proof.log.data", 32)
        topics = log.get("topics")
        if not isinstance(topics, list) or len(topics) < 3:
            raise ValueError("transfer_proof.log.topics must contain ERC-20 topics")
        for idx, topic in enumerate(topics):
            require_hex(topic, f"transfer_proof.log.topics[{idx}]", 32)


def build_fixture_from_archive(archive_path: str) -> dict[str, Any]:
    archive = load_archive(archive_path)
    finality_proof = {
        "proof_type": "ethereum-header-rlp-v1",
        "rlp_header": require_hex(archive["rlp_header"], "archive.rlp_header"),
        "header_hash": archive_header_hash(archive),
        "receipts_root": archive_receipts_root(archive),
        "source": archive.get("source", {}),
    }
    transfer_proof = {
        "proof_type": "ethereum-receipt-trie-v1",
        "receipt_key": require_hex(archive["receipt_key"], "archive.receipt_key"),
        "receipt_rlp": require_hex(archive["receipt_rlp"], "archive.receipt_rlp"),
        "receipt_hash": require_hex(archive["receipt_hash"], "archive.receipt_hash", 32),
        "receipts_root": archive_receipts_root(archive),
        "trie_nodes": [
            require_hex(node, "archive.trie_nodes[]")
            for node in archive["trie_nodes"]
        ],
        "source": archive.get("source", {}),
    }
    if "log" in archive:
        transfer_proof["log"] = archive["log"]
    validate_finality_proof(finality_proof)
    validate_transfer_proof(transfer_proof)
    return {
        "source_finality_proof": finality_proof,
        "transfer_proof": transfer_proof,
    }


def build_fixture(args: argparse.Namespace) -> dict[str, Any]:
    receipt = rpc(args.rpc_url, "eth_getTransactionReceipt", [args.tx_hash])
    if not receipt:
        raise SystemExit("transaction receipt not found")
    if receipt.get("status") != "0x1":
        raise SystemExit("transaction receipt status is not successful")

    block_hash = receipt["blockHash"]
    block = rpc(args.rpc_url, "eth_getBlockByHash", [block_hash, False])
    if not block:
        raise SystemExit("receipt block not found")

    archive = load_archive(args.receipt_proof_archive)
    receipt_key = require_hex(archive["receipt_key"], "archive.receipt_key")
    trie_nodes = [require_hex(node, "archive.trie_nodes[]") for node in archive["trie_nodes"]]
    receipt_rlp = require_hex(archive["receipt_rlp"], "archive.receipt_rlp")
    receipt_hash = require_hex(archive["receipt_hash"], "archive.receipt_hash", 32)
    rlp_header = require_hex(archive["rlp_header"], "archive.rlp_header")
    header_hash = require_hex(
        args.trusted_header_hash or archive.get("header_hash") or block["hash"],
        "header_hash",
        32,
    )
    receipts_root = require_hex(block["receiptsRoot"], "block.receiptsRoot", 32)

    finality_proof = {
        "proof_type": "ethereum-header-rlp-v1",
        "rlp_header": rlp_header,
        "header_hash": header_hash,
        "receipts_root": receipts_root,
        "source": {
            "block_hash": normalize_hex(block["hash"]),
            "block_number": block["number"],
            "transaction_hash": normalize_hex(args.tx_hash),
        },
    }
    transfer_proof = {
        "proof_type": "ethereum-receipt-trie-v1",
        "receipt_key": receipt_key,
        "receipt_rlp": receipt_rlp,
        "receipt_hash": receipt_hash,
        "receipts_root": receipts_root,
        "trie_nodes": trie_nodes,
        "source": {
            "transaction_hash": normalize_hex(args.tx_hash),
            "proof_archive": args.receipt_proof_archive,
        },
    }
    if args.token_address or args.receiver or args.amount is not None:
        if not (args.token_address and args.receiver and args.amount is not None):
            raise ValueError("--token-address, --receiver, and --amount must be provided together")
        transfer_proof["log"] = extract_transfer_log(
            receipt, args.token_address, args.receiver, args.amount
        )
    validate_finality_proof(finality_proof)
    validate_transfer_proof(transfer_proof)
    return {
        "source_finality_proof": finality_proof,
        "transfer_proof": transfer_proof,
    }


def validate_fixture(path: str) -> None:
    with open(path, "r", encoding="utf-8") as handle:
        fixture = json.load(handle)
    validate_finality_proof(fixture["source_finality_proof"])
    validate_transfer_proof(fixture["transfer_proof"])


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--validate-only", help="validate an existing fixture JSON")
    parser.add_argument(
        "--from-archive-only",
        help="emit fixture JSON directly from an archive without RPC",
    )
    parser.add_argument("--rpc-url", help="Ethereum JSON-RPC URL")
    parser.add_argument("--tx-hash", help="Ethereum transaction hash")
    parser.add_argument(
        "--receipt-proof-archive",
        help="JSON archive containing rlp_header, receipt_key, receipt_rlp, receipt_hash, trie_nodes",
    )
    parser.add_argument("--trusted-header-hash", help="override trusted finalized header hash")
    parser.add_argument("--token-address", help="ERC-20 token/log address")
    parser.add_argument("--receiver", help="ERC-20 receiver address")
    parser.add_argument("--amount", type=int, help="raw ERC-20 amount")
    parser.add_argument("--output", help="write generated fixture to this path")
    args = parser.parse_args()

    if args.validate_only:
        validate_fixture(args.validate_only)
        print(f"validated {args.validate_only}")
        return 0

    if args.from_archive_only:
        fixture = build_fixture_from_archive(args.from_archive_only)
        encoded = json.dumps(fixture, indent=2, sort_keys=True) + "\n"
        if args.output:
            with open(args.output, "w", encoding="utf-8") as handle:
                handle.write(encoded)
        else:
            sys.stdout.write(encoded)
        return 0

    required = [
        "rpc_url",
        "tx_hash",
        "receipt_proof_archive",
    ]
    missing = [field for field in required if getattr(args, field) in (None, "")]
    if missing:
        parser.error("missing required arguments: " + ", ".join("--" + m.replace("_", "-") for m in missing))

    fixture = build_fixture(args)
    encoded = json.dumps(fixture, indent=2, sort_keys=True) + "\n"
    if args.output:
        with open(args.output, "w", encoding="utf-8") as handle:
            handle.write(encoded)
    else:
        sys.stdout.write(encoded)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
