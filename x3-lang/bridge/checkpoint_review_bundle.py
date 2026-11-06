"""Read-only NEAR checkpoint candidate packaging; NEVER checkpoint approval.

Works exclusively from archived RPC JSON. No network, private keys, signing,
trust promotion, contract operations or financial authorization.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path

from near_authenticated import (
    build_checkpoint_candidate,
    checkpoint_checksum,
    checkpoint_signing_message,
    seal_checkpoint_document,
)
from near_validator_commitment import compute_next_bps_hash
from near_merkle import lite_block_hash, encode_hash

NETWORK = 'testnet'
GENESIS = 'FWJ9kR6KFWoyMoNjpLXXGHeuiy7tEY6GmoFeCA5yuc6b'
VERSION = 88
SCHEMA = 'x3.near-checkpoint-unsigned-review-bundle.v1'
PREVIOUS = 'previous-epoch-rpc.json'
CANDIDATE = 'candidate-rpc.json'
VALIDATORS = 'current-validators.json'
CHECKPOINT = 'checkpoint.unsigned.json'
MANIFEST = 'review-manifest.json'
SOURCE_SHAS = {
    PREVIOUS: 'acf673ba657bc0b90f75e17bf5fd1f407ae81103f8cbfa6f6c1fd74713f8a5e0',
    CANDIDATE: '466f916b65037b2f3d51e0958d3cb8ae1e989c4e18f8d64e07b9961939f1ffb6',
}


class BundleError(ValueError):
    pass


def digest(raw: bytes) -> str:
    return hashlib.sha256(raw).hexdigest()


def _encode(doc: dict) -> bytes:
    return (json.dumps(doc, indent=2, sort_keys=True, ensure_ascii=False, allow_nan=False) + '\n').encode('utf-8')


def _write_immutable(path: Path, raw: bytes) -> None:
    if path.exists():
        if path.read_bytes() != raw:
            raise BundleError('refusing overwrite of existing review artifact: ' + str(path))
        return
    with path.open('xb') as stream:
        stream.write(raw)


def assemble_documents(previous_bytes: bytes, candidate_bytes: bytes) -> tuple[dict, dict, dict]:
    if digest(previous_bytes) != SOURCE_SHAS[PREVIOUS]:
        raise BundleError('historical previous-epoch RPC checksum mismatch')
    if digest(candidate_bytes) != SOURCE_SHAS[CANDIDATE]:
        raise BundleError('historical candidate RPC checksum mismatch')
    prev = json.loads(previous_bytes)
    current = json.loads(candidate_bytes)
    if prev.get('jsonrpc') != '2.0' or current.get('jsonrpc') != '2.0' or prev.get('error') or current.get('error'):
        raise BundleError('invalid archived RPC envelopes')
    prev_view, view = prev.get('result'), current.get('result')
    if not isinstance(prev_view, dict) or not isinstance(view, dict):
        raise BundleError('missing archived RPC results')
    prev_inner, inner = prev_view.get('inner_lite'), view.get('inner_lite')
    if not isinstance(prev_inner, dict) or not isinstance(inner, dict):
        raise BundleError('missing block-lite fields')
    if prev_inner.get('next_epoch_id') != inner.get('epoch_id'):
        raise BundleError('previous block does not commit candidate epoch')
    if prev_inner.get('epoch_id') == inner.get('epoch_id'):
        raise BundleError('previous update must come from earlier epoch')
    if not isinstance(prev_inner.get('height'), int) or not isinstance(inner.get('height'), int) or prev_inner['height'] >= inner['height']:
        raise BundleError('non-monotonic candidate height')
    producers = prev_view.get('next_bps')
    if not isinstance(producers, list) or not producers:
        raise BundleError('no previous epoch next_bps')
    if compute_next_bps_hash(producers) != prev_inner.get('next_bp_hash'):
        raise BundleError('previous epoch validator set does not match its own header commitment')
    if encode_hash(lite_block_hash(prev_view)) == encode_hash(lite_block_hash(view)):
        raise BundleError('candidate block hash equals previous block hash')
    checkpoint = build_checkpoint_candidate(
        current, producers, network=NETWORK, genesis_hash=GENESIS,
        protocol_version=VERSION, provenance={
            'source': 'archived NEAR testnet RPC responses, PR #59 and PR #60, NOT independently authenticated',
            'verified_by': 'UNREVIEWED — independent external reviewers required',
            'observed_at': 'UNKNOWN: capture time not included in archived RPC payload; review original PR provenance',
        },
    )
    checkpoint = seal_checkpoint_document(checkpoint)
    validator_doc = {'schema':'x3.near-checkpoint-validator-list-candidate.v1','validators':producers,'authenticated':False}
    manifest = {
        'schema': SCHEMA,
        'status': 'UNSIGNED_UNREVIEWED_RPC_CANDIDATE',
        'checkpoint_approved': False,
        'authenticated': False,
        'authorized_to_sign': False,
        'network_claim': NETWORK,
        'genesis_claim': GENESIS,
        'protocol_version_claim': VERSION,
        'checkpoint_height': inner['height'],
        'checkpoint_block_hash': checkpoint['block_hash'],
        'checkpoint_checksum': checkpoint['checksum'],
        'message_hex_for_external_review_ONLY': checkpoint_signing_message(checkpoint['checksum']).hex(),
        'source_sha256': dict(SOURCE_SHAS),
        'artifact_sha256': {VALIDATORS:digest(_encode(validator_doc)),CHECKPOINT:digest(_encode(checkpoint))},
        'source_provenance': {
            PREVIOUS:'PR #60 HbM→p27 candidate; RPC result archived with normalization, one RPC source',
            CANDIDATE:'PR #59 p27 candidate; RPC capture compared across two providers in PR, independently unverified',
        },
        'required_external_review': [
            'Establish true NEAR testnet identity and canonical genesis through independently trusted channel.',
            'Independently validate originating checkpoint against trusted nearcore state or reviewed genesis trust root.',
            'Verify current epoch validator set and their chain of authorization, not only the predecessor RPC claim.',
            'Review protocol-88 and V1 Borsh serialization support against pinned nearcore.',
            'Approve exact checkpoint digest through externally pinned policy and threshold offline signatures.',
            'Record reviewer identities, independent evidence and signed approval outside this package.',
        ],
    }
    return validator_doc, checkpoint, manifest


def build(directory: Path) -> dict:
    previous_bytes = (directory / PREVIOUS).read_bytes()
    candidate_bytes = (directory / CANDIDATE).read_bytes()
    validators, checkpoint, manifest = assemble_documents(previous_bytes, candidate_bytes)
    _write_immutable(directory / VALIDATORS, _encode(validators))
    _write_immutable(directory / CHECKPOINT, _encode(checkpoint))
    _write_immutable(directory / MANIFEST, _encode(manifest))
    return verify(directory)


def verify(directory: Path) -> dict:
    previous_bytes = (directory / PREVIOUS).read_bytes()
    candidate_bytes = (directory / CANDIDATE).read_bytes()
    validators, checkpoint, manifest = assemble_documents(previous_bytes, candidate_bytes)
    for name, expected in [(VALIDATORS,validators),(CHECKPOINT,checkpoint),(MANIFEST,manifest)]:
        if (directory / name).read_bytes() != _encode(expected):
            raise BundleError('bundle verification failed: unexpected ' + name)
    current = json.loads((directory / CHECKPOINT).read_bytes())
    if checkpoint_checksum(current) != manifest['checkpoint_checksum']:
        raise BundleError('checkpoint checksum mismatch')
    if any((manifest['checkpoint_approved'],manifest['authenticated'],manifest['authorized_to_sign'])):
        raise BundleError('review bundle must remain unapproved')
    return {'status':manifest['status'],'checkpoint_height':manifest['checkpoint_height'],
            'checkpoint_checksum': manifest['checkpoint_checksum'],
            'manifest_sha256':digest((directory / MANIFEST).read_bytes()),
            'sources_verified_by_sha256':2, 'authenticated':False,'authorized_to_sign':False}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('command', choices=['build','verify'])
    parser.add_argument('--directory',type=Path,required=True)
    args = parser.parse_args()
    result = build(args.directory) if args.command == 'build' else verify(args.directory)
    print(json.dumps(result,indent=2,sort_keys=True))


if __name__ == '__main__':
    main()
