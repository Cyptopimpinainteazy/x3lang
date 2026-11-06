"""Validate NEAR V1 validator-set Borsh commitment against UNTRUSTED header.

Neither `next_bps` nor the lite header become authenticated by comparing
these two values. Cryptographic approval verification also needs an
independently authenticated validator set from the *preceding epoch*.
"""
from __future__ import annotations

import hashlib
import re
from dataclasses import dataclass

from near_merkle import decode_hash, encode_hash, lite_block_hash


@dataclass(frozen=True)
class ValidatorCommitmentCheck:
    validator_count: int
    next_bp_hash: str
    candidate_block_hash: str
    authenticated: bool = False
    authorized_to_sign: bool = False


def _n(value: int, width: int) -> bytes:
    if type(value) is not int or not 0 <= value < 1 << (8 * width):
        raise ValueError('invalid Borsh integer')
    return value.to_bytes(width, 'little')


def compute_next_bps_hash(validators: list) -> str:
    """SHA256(Borsh(Vec<ValidatorStake::V1>)) for current known V1 layout.

    Fail closed on V2 or future schema changes until their correct protocol
    encoding is independently reviewed and covered by live-chain fixtures.
    """
    if type(validators) is not list or not 1 <= len(validators) <= 4096:
        raise ValueError('empty or oversized validator list')
    raw = bytearray(_n(len(validators), 4))
    accounts = set()
    keys = set()
    for validator in validators:
        if type(validator) is not dict or validator.get('validator_stake_struct_version') != 'V1':
            raise ValueError('unsupported validator stake encoding')
        account = validator.get('account_id')
        if (type(account) is not str or not re.fullmatch(r'[a-z0-9_-]+(?:\.[a-z0-9_-]+)*', account)
                or len(account.encode()) > 64 or account in accounts):
            raise ValueError('invalid or duplicate validator account')
        accounts.add(account)
        key = validator.get('public_key')
        if type(key) is not str or not key.startswith('ed25519:'):
            raise ValueError('unsupported validator key algorithm')
        key_raw = decode_hash(key.removeprefix('ed25519:'))
        if key_raw in keys:
            raise ValueError('duplicate validator key')
        keys.add(key_raw)
        stake = validator.get('stake')
        if type(stake) is not str or not re.fullmatch(r'[1-9][0-9]*', stake):
            raise ValueError('invalid or zero validator stake')
        account_bytes = account.encode()
        # Borsh enum ValidatorStake::V1=0, account_id String, PublicKey::ED25519=0,
        # Ed25519 key length 32, then Balance u128.
        raw.extend(b'\x00' + _n(len(account_bytes), 4) + account_bytes
                   + b'\x00' + key_raw + _n(int(stake), 16))
    return encode_hash(hashlib.sha256(raw).digest())


def check_next_bps_commitment(light_client_view: dict) -> ValidatorCommitmentCheck:
    if type(light_client_view) is not dict or type(light_client_view.get('inner_lite')) is not dict:
        raise ValueError('invalid light client block view')
    header = light_client_view['inner_lite']
    computed = compute_next_bps_hash(light_client_view.get('next_bps'))
    committed = header.get('next_bp_hash')
    if computed != committed:
        raise ValueError('untrusted NEAR validator list does not match header commitment')
    # Reconstruction checks internal hash consistency only, not checkpoint truth.
    block_hash = encode_hash(lite_block_hash(light_client_view))
    return ValidatorCommitmentCheck(len(light_client_view['next_bps']), computed, block_hash)
