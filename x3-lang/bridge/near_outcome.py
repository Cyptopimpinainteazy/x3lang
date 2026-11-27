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


STATUS_UNKNOWN = 'Unknown'
STATUS_FAILURE = 'Failure'
STATUS_SUCCESS_VALUE = 'SuccessValue'
STATUS_SUCCESS_RECEIPT_ID = 'SuccessReceiptId'
OUTCOME_WITH_ID_FIELDS = frozenset(('proof', 'block_hash', 'id', 'outcome'))
OUTCOME_FIELDS = frozenset((
    'logs', 'receipt_ids', 'gas_burnt', 'tokens_burnt', 'executor_id', 'status', 'metadata',
))
PROOF_RESULT_FIELDS = frozenset((
    'outcome_proof', 'outcome_root_proof', 'block_header_lite', 'block_proof',
))
BLOCK_HEADER_LITE_FIELDS = frozenset(('prev_block_hash', 'inner_rest_hash', 'inner_lite'))


def partial_execution_status(status: object) -> tuple[str, bytes]:
    """Borsh(PartialExecutionStatus) from an RPC `ExecutionStatusView`.

    nearcore core/primitives/src/transaction.rs (592a264):
        enum PartialExecutionStatus { Unknown = 0, Failure = 1,
            SuccessValue(Vec<u8>) = 2, SuccessReceiptId(CryptoHash) = 3 }
    `Failure` drops the error payload, so the proof authenticates *that* the
    outcome failed but nothing about why. Returns (status kind, bytes).
    """
    if status == STATUS_UNKNOWN:
        return STATUS_UNKNOWN, b'\x00'
    if type(status) is not dict or len(status) != 1:
        raise OutcomeProofError('malformed NEAR execution status')
    kind, value = next(iter(status.items()))
    if kind == STATUS_FAILURE:
        if type(value) is not dict:
            raise OutcomeProofError('malformed NEAR execution failure')
        return STATUS_FAILURE, b'\x01'
    if kind == STATUS_SUCCESS_VALUE:
        if type(value) is not str or len(value) > 1_400_000:
            raise OutcomeProofError('malformed success return value')
        try:
            raw = base64.b64decode(value, validate=True)
        except (ValueError, binascii.Error) as exc:
            raise OutcomeProofError('malformed success return value') from exc
        if base64.b64encode(raw).decode('ascii') != value:
            raise OutcomeProofError('non-canonical base64 success return value')
        if len(raw) > 1_000_000:
            raise OutcomeProofError('oversized success return value')
        return STATUS_SUCCESS_VALUE, b'\x02' + _uint(len(raw), 32) + raw
    if kind == STATUS_SUCCESS_RECEIPT_ID:
        return STATUS_SUCCESS_RECEIPT_ID, b'\x03' + decode_hash(value)
    raise OutcomeProofError('unknown NEAR execution status variant')


def _partial_status(status: object) -> bytes:
    return partial_execution_status(status)[1]


def outcome_leaf(outcome_with_id: dict) -> bytes:
    """SHA256(Borsh(Vec<Hash>)): [id, hash(partial), *hash(log_bytes)].

    This is nearcore `ExecutionOutcomeWithIdView::to_hashes`. Every status
    variant is encoded; callers decide which status they accept.
    """
    if type(outcome_with_id) is not dict or type(outcome_with_id.get('outcome')) is not dict:
        raise OutcomeProofError('invalid execution outcome')
    outcome = outcome_with_id['outcome']
    if set(outcome) - OUTCOME_FIELDS or {'logs', 'receipt_ids', 'gas_burnt', 'tokens_burnt',
                                          'executor_id', 'status'} - set(outcome):
        raise OutcomeProofError('unexpected or missing execution outcome field')
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


@dataclass(frozen=True)
class OutcomeInclusion:
    """An execution outcome included under a CALLER-SUPPLIED head Merkle root.

    `status` is the authenticated PartialExecutionStatus variant only if the
    head root itself came from an authenticated light-client head.
    """
    outcome_id: str
    executor_id: str
    status: str
    block_hash: str
    shard_outcome_root: str
    block_outcome_root: str
    head_block_merkle_root: str
    authenticated: bool = False
    authorized_to_sign: bool = False


def _strict_path(path: object, label: str) -> list:
    if type(path) is not list or len(path) > 128:
        raise OutcomeProofError(f'invalid {label} Merkle path')
    for node in path:
        if type(node) is not dict or set(node) != {'hash', 'direction'}:
            raise OutcomeProofError(f'malformed {label} Merkle branch')
        if node['direction'] not in ('Left', 'Right'):
            raise OutcomeProofError(f'unknown {label} Merkle direction')
        decode_hash(node['hash'])
    return path


def verify_outcome_inclusion(proof_result: dict, *, head_block_merkle_root: bytes) -> OutcomeInclusion:
    """Verify a `light_client_proof` result for ANY outcome status.

    Mirrors near-light-client `Protocol::inclusion_proof_verify` (Basic):
      1. block_hash(block_header_lite) == outcome_proof.block_hash
      2. shard root = MerkleRoot(sha256(Borsh(to_hashes)), outcome_proof.proof)
      3. block outcome root = MerkleRoot(sha256(shard root), outcome_root_proof)
         == block_header_lite.inner_lite.outcome_root
      4. MerkleRoot(block_hash, block_proof) == head_block_merkle_root
    The root is not authenticated here; see near_authenticated for that.
    """
    try:
        if type(proof_result) is not dict or set(proof_result) != PROOF_RESULT_FIELDS:
            raise OutcomeProofError('unexpected or missing light_client_proof field')
        if type(head_block_merkle_root) is not bytes or len(head_block_merkle_root) != 32:
            raise OutcomeProofError('invalid head Merkle root')
        proof = proof_result['outcome_proof']
        if type(proof) is not dict or set(proof) != OUTCOME_WITH_ID_FIELDS:
            raise OutcomeProofError('unexpected or missing outcome_proof field')
        lite = proof_result['block_header_lite']
        if type(lite) is not dict or set(lite) != BLOCK_HEADER_LITE_FIELDS:
            raise OutcomeProofError('unexpected or missing block_header_lite field')
        outcome_path = _strict_path(proof['proof'], 'outcome')
        root_path = _strict_path(proof_result['outcome_root_proof'], 'outcome root')
        block_path = _strict_path(proof_result['block_proof'], 'block')
        status, _ = partial_execution_status(proof['outcome'].get('status'))
        executor = proof['outcome'].get('executor_id')
        if type(executor) is not str:
            raise OutcomeProofError('invalid executor')
        block_hash = lite_block_hash(lite)
        if block_hash != decode_hash(proof['block_hash']):
            raise OutcomeProofError('execution block hash not bound to lite header')
        shard_root = compute_merkle_root(outcome_leaf(proof), outcome_path)
        block_outcome_root = compute_merkle_root(hashlib.sha256(shard_root).digest(), root_path)
        if block_outcome_root != decode_hash(lite['inner_lite']['outcome_root']):
            raise OutcomeProofError('execution outcome not included in block header')
        if compute_merkle_root(block_hash, block_path) != head_block_merkle_root:
            raise OutcomeProofError('execution block not included under head Merkle root')
        return OutcomeInclusion(
            outcome_id=encode_hash(decode_hash(proof['id'])),
            executor_id=executor,
            status=status,
            block_hash=encode_hash(block_hash),
            shard_outcome_root=encode_hash(shard_root),
            block_outcome_root=encode_hash(block_outcome_root),
            head_block_merkle_root=encode_hash(head_block_merkle_root),
        )
    except OutcomeProofError:
        raise
    except (TypeError, KeyError, IndexError, AttributeError, ValueError) as exc:
        raise OutcomeProofError('invalid or unverified NEAR outcome proof') from exc
