"""NEAR light-client hashing and Merkle path *primitives*.

These computations authenticate nothing on their own. A verified NEAR
checkpoint and validated validator-signature epoch transitions are REQUIRED
before their outputs can constitute chain evidence. No signing interface.
"""
from __future__ import annotations

from hashlib import sha256

ALPHABET = '123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz'


def decode_hash(value: str) -> bytes:
    if type(value) is not str or not 32 <= len(value) <= 48:
        raise ValueError('invalid NEAR hash encoding')
    acc = 0
    for char in value:
        if char not in ALPHABET:
            raise ValueError('invalid base58 alphabet')
        acc = acc * 58 + ALPHABET.index(char)
    zero_prefix = len(value) - len(value.lstrip('1'))
    result = b'\x00' * zero_prefix + (acc.to_bytes((acc.bit_length() + 7) // 8, 'big') if acc else b'')
    if len(result) != 32 or encode_hash(result) != value:
        raise ValueError('NEAR hash must decode canonically to 32 bytes')
    return result


def encode_hash(value: bytes) -> str:
    if type(value) is not bytes or len(value) != 32:
        raise ValueError('expected 32-byte hash')
    n = int.from_bytes(value, 'big')
    symbols = []
    while n:
        n, i = divmod(n, 58)
        symbols.append(ALPHABET[i])
    return '1' * (len(value) - len(value.lstrip(b'\x00'))) + ''.join(reversed(symbols))


def _sha(value: bytes) -> bytes:
    return sha256(value).digest()


def compute_merkle_root(leaf: bytes, path: list[dict]) -> bytes:
    """Compute NEAR's binary Merkle root: SHA256(left || right)."""
    if type(leaf) is not bytes or len(leaf) != 32 or type(path) is not list or len(path) > 128:
        raise ValueError('invalid Merkle leaf/path')
    current = leaf
    for node in path:
        if type(node) is not dict or set(node) != {'hash', 'direction'}:
            raise ValueError('malformed Merkle branch')
        sibling = decode_hash(node['hash'])
        if node['direction'] == 'Left':
            current = _sha(sibling + current)
        elif node['direction'] == 'Right':
            current = _sha(current + sibling)
        else:
            raise ValueError('unknown Merkle branch direction')
    return current


def verify_block_inclusion(block_hash: bytes, block_proof: list[dict], trusted_head_block_merkle_root: bytes) -> bool:
    """Merkle membership relative to a caller-supplied root, NOT chain finality.

    The root MUST be independently authenticated from a verified light-client
    head before this boolean has security meaning. Do not use this to sign.
    """
    if type(trusted_head_block_merkle_root) is not bytes or len(trusted_head_block_merkle_root) != 32:
        raise ValueError('invalid expected Merkle root')
    return compute_merkle_root(block_hash, block_proof) == trusted_head_block_merkle_root


def lite_block_hash(header: dict) -> bytes:
    """Nomicon block-lite hash from Borsh inner_lite + supplied inner_rest hash.

    Header contents and inner_rest_hash are still untrusted until block inclusion
    against an authenticated final light-client head is independently verified.
    """
    if type(header) is not dict:
        raise ValueError('missing header')
    inner = header.get('inner_lite')
    if type(inner) is not dict:
        raise ValueError('missing inner_lite')
    height = inner.get('height')
    timestamp = inner.get('timestamp')
    if (type(height) is not int or not 0 <= height < 2**64 or
            type(timestamp) is not int or not 0 <= timestamp < 2**64):
        raise ValueError('invalid Borsh u64 field')
    borsh = b''.join((
        height.to_bytes(8, 'little'),
        decode_hash(inner.get('epoch_id')),
        decode_hash(inner.get('next_epoch_id')),
        decode_hash(inner.get('prev_state_root')),
        decode_hash(inner.get('outcome_root')),
        timestamp.to_bytes(8, 'little'),
        decode_hash(inner.get('next_bp_hash')),
        decode_hash(inner.get('block_merkle_root')),
    ))
    return _sha(_sha(_sha(borsh) + decode_hash(header.get('inner_rest_hash'))) + decode_hash(header.get('prev_block_hash')))

from dataclasses import dataclass


@dataclass(frozen=True)
class BlockMerkleCheck:
    """Cryptographic tree membership relative to a CALLER-SUPPLIED root only."""
    block_hash: str
    merkle_path_matches: bool
    authenticated: bool = False
    authorized_to_sign: bool = False


def check_receipt_block_merkle(response: dict, *, receipt_id: str, receiver_id: str,
                               light_client_head: str,
                               expected_head_block_merkle_root: bytes) -> BlockMerkleCheck:
    """Verify block-header hashing and ancestry proof, never validator finality.

    The expected head root is *not* authenticated here. Without independent
    verification of the validator-approved light-client head, this must not
    be used to authorize a trade or attest an escrow payout. Execution
    outcome inclusion and NEAR state proofs are also NOT checked here.
    """
    from near_receipt_proof import get_candidate

    candidate = get_candidate(response, receipt_id=receipt_id,
                              receiver_id=receiver_id, light_client_head=light_client_head)
    result = response['result']
    calculated_hash = lite_block_hash(result['block_header_lite'])
    if decode_hash(candidate.block_hash) != calculated_hash:
        raise ValueError('outcome proof block hash disagrees with lite header hash')
    if not verify_block_inclusion(calculated_hash, result['block_proof'],
                                  expected_head_block_merkle_root):
        raise ValueError('block proof does not match expected head merkle root')
    return BlockMerkleCheck(block_hash=candidate.block_hash, merkle_path_matches=True)
