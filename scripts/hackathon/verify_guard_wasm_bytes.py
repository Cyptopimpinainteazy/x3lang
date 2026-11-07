#!/usr/bin/env python3
"""Offline, fail-closed deployed Guard WASM reproducibility verifier.

This proves ONLY byte equality to a source-pinned SHA256 and the archived
public-RPC code hash. It does not independently authenticate NEAR consensus,
prove contract execution, authorize signing or transmit a transaction.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import sys
from pathlib import Path

DEPLOYED_SHA256 = 'ac44fa53acf514d26c7adf883b31b49a125faf5a4c1074928d3e1fe94b36f8ab'
DEPLOYED_CODE_HASH = 'CbU8SpgfTgG1egbw8ygMTK3deozvDnoFrF8DJ8qifSe6'
ACCOUNT = 'x3lang-guard-20261106.testnet'
BASE58 = '123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz'


class IntegrityError(ValueError):
    pass


def b58encode(raw: bytes) -> str:
    count = len(raw) - len(raw.lstrip(b'\0'))
    number = int.from_bytes(raw, 'big')
    digits: list[str] = []
    while number:
        number, mod = divmod(number, 58)
        digits.append(BASE58[mod])
    return '1' * count + ''.join(reversed(digits))


def canonical_hash(value: dict) -> str:
    raw = json.dumps(value, sort_keys=True, separators=(',', ':'), ensure_ascii=False).encode()
    return hashlib.sha256(raw).hexdigest()


def verify_bytes(manifest: dict, wasm: bytes, *, expected_sha: str) -> dict:
    if not isinstance(manifest, dict):
        raise IntegrityError('manifest must be a JSON object')
    manifest_body = {key: val for key, val in manifest.items() if key != 'record_hash'}
    if manifest.get('record_hash') != canonical_hash(manifest_body):
        raise IntegrityError('manifest record_hash mismatch')
    if (manifest.get('schema') != 'x3.guard.testnet.manifest.v1'
            or manifest.get('chain_id') != 'testnet'
            or manifest.get('account_id') != ACCOUNT):
        raise IntegrityError('manifest chain/account/schema mismatch')
    if not isinstance(expected_sha, str) or len(expected_sha) != 64:
        raise IntegrityError('invalid pinned SHA256')
    if manifest.get('wasm_sha256') != expected_sha:
        raise IntegrityError('manifest does not match pinned deployment SHA256')
    if len(wasm) < 8 or not wasm.startswith(b'\0asm'):
        raise IntegrityError('artifact is not a WebAssembly module')
    if len(wasm) != manifest.get('wasm_bytes'):
        raise IntegrityError('compiled WASM byte count does not match manifest')
    digest = hashlib.sha256(wasm).digest()
    if digest.hex() != expected_sha:
        raise IntegrityError('compiled WASM SHA256 does not match pinned deployment')
    code_hash = b58encode(digest)
    if manifest.get('code_hash') != code_hash:
        raise IntegrityError('manifest code_hash does not equal base58(WASM SHA256)')
    return {
        'account': ACCOUNT,
        'wasm_bytes': len(wasm),
        'wasm_sha256': digest.hex(),
        'code_hash': code_hash,
        'source_binary_matches_pinned_deployed_wasm': True,
        'independent_consensus_finality_established': False,
        'signing_authorized': False,
        'funds_moved': False,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--manifest', type=Path, required=True)
    parser.add_argument('--wasm', type=Path, required=True)
    args = parser.parse_args()
    try:
        manifest = json.loads(args.manifest.read_text())
        raw = args.wasm.read_bytes()
        result = verify_bytes(manifest, raw, expected_sha=DEPLOYED_SHA256)
        if result['code_hash'] != DEPLOYED_CODE_HASH:
            raise IntegrityError('computed code hash disagrees with hard-pinned on-chain observation')
    except (IntegrityError, OSError, ValueError, TypeError) as exc:
        print(f'BLOCKED: Guard WASM byte reproducibility check: {exc}', file=sys.stderr)
        return 1
    print('EXACT_GUARD_WASM_BYTES_MATCH_DEPLOYED_HASH')
    for key in ('account','wasm_bytes','wasm_sha256','code_hash'):
        print(f'{key}={result[key]}')
    print('CHAIN_FINALITY_AUTHENTICATED=false SIGNING_AUTHORIZED=false FUNDS_MOVED=false')
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
