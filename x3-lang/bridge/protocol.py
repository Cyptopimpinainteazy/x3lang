"""Non-custodial swap parameter builder and *unverified* progress gate.

The source-chain lock must finalize before the destination provider locks NEAR.
The recipient claims NEAR's contract credit before revealing the preimage to
unlock native BNB. Source expiry must be later than destination expiry.
No keys, deposit addresses, price feeds or broadcast actions are implemented.
"""
from __future__ import annotations

from dataclasses import dataclass
import hashlib
import json
import re


class ProtocolError(ValueError):
    pass


@dataclass(frozen=True)
class Plan:
    swap_id_hex: str
    hashlock_hex: str
    initiator_bsc: str
    lp_bsc: str
    recipient_near: str
    lp_near: str
    bsc_escrow: str
    near_vault: str
    nonce_hex: str
    bnb_wei: int
    near_yocto: int
    bsc_expiry_sec: int
    near_funded_before_ns: int
    near_expiry_ns: int
    bsc_chain_id: int = 56
    near_network: str = "mainnet"
    destination_chain: str = "near"
    status: str = "UNFUNDED_PLAN"


def create_plan(*, secret: bytes, initiator_bsc: str, lp_bsc: str,
                recipient_near: str, lp_near: str, bsc_escrow: str,
                near_vault: str, nonce_hex: str, bnb_wei: int,
                near_yocto: int, bsc_expiry_sec: int,
                near_funded_before_sec: int, near_expiry_sec: int,
                now_sec: int, min_safety_sec: int = 3600,
                bsc_chain_id: int = 56, near_network: str = "mainnet") -> Plan:
    if (min_safety_sec < 3600 or near_network not in ("testnet", "mainnet") or
            bsc_chain_id not in (56, 97) or
            (near_network == "testnet") != (bsc_chain_id == 97)):
        raise ProtocolError("inconsistent chain network or unsafe minimum safety window")
    if not isinstance(secret, bytes) or len(secret) != 32:
        raise ProtocolError("secret must be exactly 32 cryptographically random bytes")
    for addr in (initiator_bsc, lp_bsc, bsc_escrow):
        if not isinstance(addr, str) or re.fullmatch(r"0x[0-9a-fA-F]{40}", addr) is None:
            raise ProtocolError("invalid 20-byte BSC address")
        if int(addr[2:], 16) == 0:
            raise ProtocolError("zero BSC identity")
    if initiator_bsc.lower() == lp_bsc.lower():
        raise ProtocolError("BSC settlement parties must be distinct")
    for account in (recipient_near, lp_near, near_vault):
        if not isinstance(account, str) or len(account) > 64 or not re.fullmatch(
            r"(?:[a-z0-9_-]+\.)*[a-z0-9_-]+", account
        ):
            raise ProtocolError("invalid NEAR account identifier")
    if recipient_near == lp_near:
        raise ProtocolError("NEAR settlement parties must be distinct")
    if near_vault in (recipient_near, lp_near):
        raise ProtocolError("NEAR vault must be distinct from settlement parties")
    if not isinstance(nonce_hex, str) or re.fullmatch(r"[0-9a-f]{64}", nonce_hex) is None:
        raise ProtocolError("nonce must be canonical 32-byte lowercase hexadecimal")
    if not all(isinstance(x, int) and not isinstance(x, bool) for x in
               (bnb_wei, near_yocto, bsc_expiry_sec, near_funded_before_sec,
                near_expiry_sec, now_sec)):
        raise ProtocolError("amounts and timeouts must be integers")
    if bnb_wei <= 0 or near_yocto <= 0:
        raise ProtocolError("both native-asset amounts must be positive")
    if now_sec <= 0:
        raise ProtocolError("invalid observation time")
    max_seconds = (2**64 - 1) // 1_000_000_000
    if max(bsc_expiry_sec, near_funded_before_sec, near_expiry_sec, now_sec) > max_seconds:
        raise ProtocolError("deadline does not fit NEAR u64 nanoseconds or BSC u64 seconds")
    if near_funded_before_sec - now_sec < 600:
        raise ProtocolError("destination funding deadline is too close")
    if near_expiry_sec - near_funded_before_sec < 600:
        raise ProtocolError("destination funding deadline leaves no claim window")
    if near_expiry_sec - now_sec > 7 * 86400:
        raise ProtocolError("destination expiry outside allowed window")
    if (bsc_expiry_sec - near_expiry_sec < min_safety_sec
            or bsc_expiry_sec - now_sec > 7 * 86400):
        raise ProtocolError("insufficient staggered expiry safety window")
    h = hashlib.sha256(secret).hexdigest()
    fields = {
        "domain": "x3.native.bsc-near.htlc.v1", "chain_id": bsc_chain_id, "near_network": near_network,
        "hashlock": h, "initiator_bsc": initiator_bsc.lower(), "lp_bsc": lp_bsc.lower(),
        "recipient_near": recipient_near, "lp_near": lp_near,
        "bsc_escrow": bsc_escrow.lower(), "near_vault": near_vault, "nonce_hex": nonce_hex,
        "bnb_wei": str(bnb_wei), "near_yocto": str(near_yocto),
        "bsc_expiry_sec": bsc_expiry_sec,
        "near_funded_before_sec": near_funded_before_sec,
        "near_expiry_sec": near_expiry_sec,
    }
    identity = hashlib.sha256(json.dumps(fields, sort_keys=True,
                            separators=(",", ":")).encode("utf-8")).hexdigest()
    return Plan(identity, h, initiator_bsc, lp_bsc, recipient_near, lp_near,
                bsc_escrow, near_vault, nonce_hex, bnb_wei, near_yocto,
                bsc_expiry_sec,
                near_funded_before_sec * 1_000_000_000,
                near_expiry_sec * 1_000_000_000,
                bsc_chain_id=bsc_chain_id, near_network=near_network)


def describe_progress(plan: Plan, *, source_claimed: bool, near_claim_credited: bool,
                      source_rpc_final: bool, near_rpc_final: bool) -> dict:
    """Preliminary state classification, NOT a substitute for RPC proof verification.

    Even all-true values are observer-supplied, not independently verified.
    No source tx hash, NEAR receipts, BSC block proofs, or deployed contract
    addresses are checked here; no settled verdict is ever issued.
    """
    if not (source_rpc_final and near_rpc_final):
        status = "UNVERIFIED"
    elif source_claimed and not near_claim_credited:
        status = "SECURITY_HALT"
    else:
        status = "OBSERVED_NOT_PROVEN"
    return {
        "status": status,
        "settled": False,
        "swap_id_hex": plan.swap_id_hex,
        "requires": ["BSC contract receipt and finalized chain proof",
                     "NEAR contract credit proof at finalized block",
                     "verified deployed contract identities",
                     "source and destination amount/hashlock matching"],
        "warning": "No chain-verifier integration yet. Never release funds based on this result.",
    }
