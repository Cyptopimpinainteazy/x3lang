"""Real NEAR execution-outcome Merkle verification WITHOUT consensus authority.

Reconstructs the version-independent PartialExecutionOutcome Borsh hash,
Merkle leaf, shard outcome root and block outcome root as nearcore does.
The block ancestry comparison is against a caller-supplied, UNTRUSTED root;
this module has no checkpoint, validator epoch verification or signing rights.
"""
from __future__ import annotations

import base64
import binascii
import hashlib
import re
from dataclasses import dataclass

from near_merkle import decode_hash, encode_hash, lite_block_hash, compute_merkle_root
from near_receipt_proof import get_candidate, ReceiptProofError


class OutcomeProofError(ValueError):
    pass


def _uint(value: object, bits: int) -> bytes:
    if type(value) is not int or not 0 <= value < (1 << bits):
        raise OutcomeProofError('invalid Borsh unsigned integer')
    return value.to_bytes(bits // 8, 'little')


def _str(value: object) -> bytes:
    if type(value) is not str or len(value.encode('utf-8')) > 1_000_000:
        raise OutcomeProofError('invalid Borsh string')
    raw = value.encode('utf-8')
    return _uint(len(raw), 32) + raw


def _partial_status(status: object) -> bytes:
    # Only successful outcomes are currently admitted; failures must never be
    # considered proof of successful escrow settlement.
    if type(status) is not dict or len(status) != 1:
        raise OutcomeProofError('unknown or unsuccessful NEAR receipt status')
    if 'SuccessValue' in status and type(status['SuccessValue']) is str:
        try:
            value = base64.b64decode(status['SuccessValue'], validate=True)
        except (ValueError, binascii.Error) as exc:
            raise OutcomeProofError('malformed success return value') from exc
        if len(value) > 1_000_000:
            raise OutcomeProofError('oversized success return value')
        return b'\x02' + _uint(len(value), 32) + value
    if 'SuccessReceiptId' in status:
        return b'\x03' + decode_hash(status['SuccessReceiptId'])
    raise OutcomeProofError('unknown or unsuccessful NEAR receipt status')


def outcome_leaf(outcome_with_id: dict) -> bytes:
    """SHA256(Borsh(Vec<Hash>)): [id, hash(partial), *hash(log_bytes)]."""
    if type(outcome_with_id) is not dict or type(outcome_with_id.get('outcome')) is not dict:
        raise OutcomeProofError('invalid execution outcome')
    outcome = outcome_with_id['outcome']
    ids = outcome.get('receipt_ids')
    logs = outcome.get('logs')
    if type(ids) is not list or len(ids) > 4096 or type(logs) is not list or len(logs) > 4096:
        raise OutcomeProofError('unbounded outcome receipts or logs')
    tokens = outcome.get('tokens_burnt')
    if type(tokens) is not str or not re.fullmatch(r'(?:0|[1-9][0-9]*)', tokens):
        raise OutcomeProofError('invalid decimal tokens burnt')
    receipt_ids = b''.join(decode_hash(item) for item in ids)
    partial = (
        _uint(len(ids), 32) + receipt_ids
        + _uint(outcome.get('gas_burnt'), 64)
        + _uint(int(tokens), 128)
        + _str(outcome.get('executor_id'))
        + _partial_status(outcome.get('status'))
    )
    log_hashes = []
    for log in logs:
        if type(log) is not str or len(log.encode('utf-8')) > 1_000_000:
            raise OutcomeProofError('malformed or oversized execution log')
        log_hashes.append(hashlib.sha256(log.encode('utf-8')).digest())
    hashes = [decode_hash(outcome_with_id.get('id')), hashlib.sha256(partial).digest(), *log_hashes]
    return hashlib.sha256(_uint(len(hashes), 32) + b''.join(hashes)).digest()


@dataclass(frozen=True)
class OutcomeInclusionCheck:
    receipt_id: str
    receiver_id: str
    block_hash: str
    shard_outcome_root: str
    block_outcome_root: str
    outcome_merkle_matches: bool
    block_merkle_matches: bool
    authenticated: bool = False
    authorized_to_sign: bool = False


def verify_receipt_outcome(
    rpc_response: dict, *, receipt_id: str, receiver_id: str,
    light_client_head: str, head_block_merkle_root: bytes,
) -> OutcomeInclusionCheck:
    """Verify outcome + block inclusion relative to an UNTRUSTED input root.

    The head Merkle root must later be bound to a independently trusted chain
    checkpoint / validator-approved finality head. A matching proof is NOT
    permission to generate a settlement certificate. A successful receipt
    also DOES NOT prove an escrow storage value or X3 vault code identity.
    """
    try:
        candidate = get_candidate(rpc_response, receipt_id=receipt_id,
                                  receiver_id=receiver_id, light_client_head=light_client_head)
        if type(head_block_merkle_root) is not bytes or len(head_block_merkle_root) != 32:
            raise OutcomeProofError('invalid, unauthenticated head Merkle root')
        result = rpc_response['result']
        proof = result['outcome_proof']
        lite = result['block_header_lite']
        expected_block = decode_hash(candidate.block_hash)
        actual_block = lite_block_hash(lite)
        if actual_block != expected_block:
            raise OutcomeProofError('execution block hash not bound to lite header')
        outcome_root = compute_merkle_root(outcome_leaf(proof), proof['proof'])
        # Block's outcome root is Merkle over SHA256(Borsh(shard_outcome_root)).
        block_outcome_root = compute_merkle_root(hashlib.sha256(outcome_root).digest(),
                                                 result['outcome_root_proof'])
        if block_outcome_root != decode_hash(lite['inner_lite']['outcome_root']):
            raise OutcomeProofError('execution outcome not included in block header')
        block_root = compute_merkle_root(actual_block, result['block_proof'])
        if block_root != head_block_merkle_root:
            raise OutcomeProofError('execution block not included in supplied light head root')
        return OutcomeInclusionCheck(
            receipt_id=receipt_id, receiver_id=receiver_id, block_hash=candidate.block_hash,
            shard_outcome_root=encode_hash(outcome_root),
            block_outcome_root=encode_hash(block_outcome_root),
            outcome_merkle_matches=True, block_merkle_matches=True,
        )
    except (TypeError, KeyError, IndexError, ReceiptProofError) as exc:
        raise OutcomeProofError('invalid or unverified NEAR outcome proof') from exc
