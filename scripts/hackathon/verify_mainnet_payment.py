#!/usr/bin/env python3
"""Read-only 1Click payment evidence for the owner-funded native USDC route."""

import argparse
import base64
import json
import os
import re
from urllib.parse import urlsplit

from mainnet_agent_preflight import API, NEAR_GENESIS, NEAR_USDC, SOL_ASSET, SOL_MINT, ni

ce = ni.chain_evidence
SOL_RPC = ce.DEFAULT_SOLANA_RPC_URLS[0]
SOL_GENESIS = "5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d"
NEAR_ARCHIVAL_RPC = "https://archival-rpc.mainnet.near.org"
INTENTS_CONTRACT = "intents.near"
SPL_TOKEN = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA"
BASE58 = r"[1-9A-HJ-NP-Za-km-z]"


class ProofRejected(ValueError):
    pass


def require(condition, reason):
    if not condition:
        raise ProofRejected(reason)


def units(value):
    require(isinstance(value, str) and re.fullmatch(r"[1-9][0-9]{0,38}", value),
            "invalid token amount")
    return int(value)


def success_value(status):
    if not isinstance(status, dict) or set(status) != {"SuccessValue"}:
        return False
    value = status["SuccessValue"]
    if not isinstance(value, str):
        return False
    try:
        # NEAR encodes successful return bytes as base64, including "" for void.
        # Reject malformed/noncanonical encodings rather than trusting a key name.
        decoded = base64.b64decode(value, validate=True)
        return base64.b64encode(decoded).decode("ascii") == value
    except ValueError:
        return False


def source_transfer(tx, signature, funding_owner, destination, amount, start, deadline):
    facts = ce.solana_transfer_facts(tx, signature=signature, mint=SOL_MINT, owner=destination)
    require(facts.get("signature_matches") and facts.get("succeeded") and tx["meta"]["err"] is None,
            "Solana deposit missing or unsuccessful")
    timestamp = facts["block_time"]
    require(type(timestamp) is int and type(facts["slot"]) is int and facts["slot"] > 0
            and start <= timestamp <= deadline,
            "Solana deposit outside the signed quote window")
    require(facts["received"] == amount
            and ce.spl_balance_delta(tx, mint=SOL_MINT, owner=funding_owner) == -amount,
            "Solana deposit amount or funding owner mismatch")
    message = tx["transaction"]["message"]
    require(any(key["pubkey"] == funding_owner and key["signer"] is True
                for key in message["accountKeys"]), "funding owner did not sign the Solana deposit")
    owners = {}
    for balance in tx["meta"]["preTokenBalances"] + tx["meta"]["postTokenBalances"]:
        if balance["mint"] == SOL_MINT:
            account = message["accountKeys"][balance["accountIndex"]]["pubkey"]
            identity = (balance["owner"], balance["uiTokenAmount"]["decimals"])
            require(account not in owners or owners[account] == identity,
                    "Solana token account ownership changed")
            owners[account] = identity
    matches = []
    for instruction in message["instructions"]:
        parsed = instruction.get("parsed")
        if instruction.get("programId") != SPL_TOKEN or not isinstance(parsed, dict):
            continue
        if parsed.get("type") not in {"transfer", "transferChecked"}:
            continue
        info = parsed["info"]
        if (owners.get(info["source"]) == (funding_owner, 6)
                and owners.get(info["destination"]) == (destination, 6)):
            quantity = info.get("tokenAmount", {}).get("amount", info.get("amount"))
            require(info.get("authority") == funding_owner
                    and info.get("mint", SOL_MINT) == SOL_MINT and units(quantity) == amount,
                    "Solana transfer instruction differs from the reviewed deposit")
            matches.append(info)
    require(len(matches) == 1, "expected one direct SPL USDC deposit")
    return {"signature": signature, "mint": SOL_MINT, "source_owner": funding_owner,
            "destination_owner": destination, "source_token_account": matches[0]["source"],
            "destination_token_account": matches[0]["destination"], "amount_units": amount,
            "slot": facts["slot"], "timestamp": timestamp, "rpc_result_sha256": ce.sha256_json(tx)}


def destination_transfer(rpc, tx_hash, signer, recipient, minimum, actual, source_time):
    network = rpc("status", [])
    require(network["chain_id"] == "mainnet" and network["genesis_hash"] == NEAR_GENESIS,
            "NEAR RPC is not mainnet")
    result = rpc("EXPERIMENTAL_tx_status", {"tx_hash": tx_hash, "sender_account_id": signer,
                                            "wait_until": "FINAL"})
    require(result["final_execution_status"] == "FINAL"
            and result["transaction"]["hash"] == tx_hash
            and result["transaction"]["signer_id"] == signer
            and success_value(result["status"]), "NEAR payout transaction not finalized successfully")
    root = result["transaction_outcome"]
    require(root["id"] == tx_hash, "NEAR transaction outcome hash mismatch")
    outcomes = {row["id"]: row for row in result["receipts_outcome"]}
    require(len(outcomes) == len(result["receipts_outcome"]), "duplicate NEAR receipt outcome")
    pending = list(root["outcome"]["receipt_ids"])
    linked = set()
    while pending:
        receipt_id = pending.pop()
        require(receipt_id not in linked, "duplicate linked NEAR receipt")
        linked.add(receipt_id)
        pending.extend(outcomes[receipt_id]["outcome"]["receipt_ids"])
    matches = []
    for receipt in result["receipts"]:
        if receipt["receiver_id"] != NEAR_USDC or receipt["predecessor_id"] != INTENTS_CONTRACT:
            continue
        for action in receipt["receipt"].get("Action", {}).get("actions", []):
            call = action.get("FunctionCall", {}) if isinstance(action, dict) else {}
            if call.get("method_name") != "ft_transfer":
                continue
            args = json.loads(base64.b64decode(call["args"], validate=True))
            if args["receiver_id"] == recipient:
                matches.append((receipt["receipt_id"], units(args["amount"])))
    require(len(matches) == 1, "expected one direct NEP-141 USDC payout receipt")
    receipt_id, amount = matches[0]
    require(receipt_id in linked and minimum <= amount == actual, "NEAR payout linkage or amount mismatch")
    row = outcomes[receipt_id]
    require(row["outcome"]["executor_id"] == NEAR_USDC and success_value(row["outcome"]["status"]),
            "NEAR USDC transfer receipt did not execute successfully")
    block = rpc("block", {"block_id": row["block_hash"]})["header"]
    final = rpc("block", {"finality": "final"})["header"]
    canonical = rpc("block", {"block_id": block["height"]})["header"]
    require(block["hash"] == row["block_hash"] == canonical["hash"]
            and 0 < block["height"] <= final["height"]
            and int(block["timestamp_nanosec"]) // 10**9 >= source_time,
            "NEAR payout receipt is noncanonical, unfinalized or precedes deposit")
    return {"transaction_hash": tx_hash, "receipt_id": receipt_id, "token": NEAR_USDC,
            "recipient": recipient, "amount_units": amount, "block_hash": block["hash"],
            "block_height": block["height"], "rpc_result_sha256": ce.sha256_json(result)}


def verify(receipt, approved_quote_hash, funding_owner, payout_signer, *, client, sol_rpc, near_rpc):
    quote = ni._signed_quote(receipt)
    require(receipt["quote_hash"] == approved_quote_hash, "quote hash differs from owner review")
    response = receipt["quote_response"]
    request = response["quoteRequest"]
    require(request["originAsset"] == SOL_ASSET and request["destinationAsset"] == "nep141:" + NEAR_USDC
            and request["swapType"] == "EXACT_INPUT" and request["recipientType"] == "DESTINATION_CHAIN"
            and request["depositType"] == "ORIGIN_CHAIN"
            and request["refundType"] == "ORIGIN_CHAIN"
            and not request.get("customRecipientMsg") and not quote.get("customRecipientMsg")
            and not quote.get("depositMemo"),
            "unsupported route, custom recipient message or required memo")
    signature = receipt["deposit"]["tx_hash"]
    require(re.fullmatch(BASE58 + "{64,90}", signature)
            and ni.registry.is_valid_receiver("solana", funding_owner)
            and funding_owner != quote["depositAddress"], "invalid Solana signature or funding owner")
    amount = units(quote["amountIn"])
    require(amount == units(request["amount"]), "quote changed the requested input amount")
    require(sol_rpc("getGenesisHash", []) == SOL_GENESIS, "Solana RPC is not mainnet-beta")
    tx = sol_rpc("getTransaction", [signature, {"encoding": "jsonParsed",
                 "commitment": "finalized", "maxSupportedTransactionVersion": 0}])
    source = source_transfer(tx, signature, funding_owner, quote["depositAddress"], amount,
                             int(ni.parse_timestamp(response["timestamp"]).timestamp()),
                             ni.parse_timestamp(quote["deadline"]).timestamp())
    status = client.status(quote["depositAddress"])
    echoed = status["quoteResponse"]
    require(ni.check_oneclick_quote_signature(echoed)[0]
            and ni.oneclick_quote_hash(echoed) == receipt["quote_hash"],
            "provider status did not echo the same authenticated quote")
    require(status["status"] == "SUCCESS", "provider has not reported a successful payout")
    details = status["swapDetails"]
    origins = details["originChainTxHashes"]
    destinations = details["destinationChainTxHashes"]
    require(len(origins) == len(destinations) == 1 and origins[0]["hash"] == signature
            and units(details["amountIn"]) == amount, "provider transaction association is ambiguous")
    tx_hash = destinations[0]["hash"]
    require(re.fullmatch(BASE58 + "{32,44}", tx_hash)
            and ni.registry.is_valid_receiver("near", payout_signer),
            "invalid NEAR payout transaction hash or signer")
    payout = destination_transfer(near_rpc, tx_hash, payout_signer, request["recipient"],
                                  units(quote["minAmountOut"]), units(details["amountOut"]),
                                  source["timestamp"])
    return {"status": "matching_finalized_transfers", "quote_signature_verified": True,
            "quote_hash": receipt["quote_hash"], "provider_status_sha256": ce.sha256_json(status),
            "provider_linkage": "unsigned_status_association",
            "cross_chain_causality_proven": False, "guard_authorization_proven": False,
            "source": source, "payout": payout}


def near_rpc_endpoint():
    configured = os.environ.get("X3_NEAR_ARCHIVAL_RPC_URL")
    if not configured:
        return NEAR_ARCHIVAL_RPC, NEAR_ARCHIVAL_RPC
    try:
        parts = urlsplit(configured)
        valid = (parts.scheme == "https" and parts.hostname and not parts.username
                 and parts.password is None and not parts.fragment
                 and not any(char.isspace() for char in configured))
    except ValueError:
        valid = False
    require(valid, "X3_NEAR_ARCHIVAL_RPC_URL must be an HTTPS RPC URL")
    return configured, "operator_configured_https"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--receipt", required=True, help="private local live quote receipt with deposit hash")
    parser.add_argument("--approved-quote-hash", required=True, help="quote hash from owner review")
    parser.add_argument("--funding-owner", required=True, help="reviewed public Solana wallet address")
    parser.add_argument("--payout-signer", required=True, help="public signer of the candidate NEAR transaction")
    args = parser.parse_args()
    try:
        endpoint, endpoint_label = near_rpc_endpoint()
        with open(args.receipt, encoding="utf-8") as handle:
            receipt = json.load(handle)
        def sol_rpc(method, params):
            return ce.json_rpc(ni.urllib_transport, SOL_RPC, method, params)

        def near_rpc(method, params):
            return ce.json_rpc(ni.urllib_transport, endpoint, method, params)

        result = verify(receipt, args.approved_quote_hash, args.funding_owner, args.payout_signer, client=ni.OneClickClient(base_url=API, jwt=""),
                        sol_rpc=sol_rpc, near_rpc=near_rpc)
    except (ValueError, KeyError, TypeError, AttributeError, IndexError, OSError,
            ni.NearIntentsError, ce.ChainEvidenceError) as exc:
        reason = str(exc) if isinstance(exc, ProofRejected) else type(exc).__name__
        print(json.dumps({"status": "unverified", "reason": reason}))
        return 5
    result["rpc_endpoints"] = {"solana": SOL_RPC, "near": endpoint_label}
    print(json.dumps(result, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
