"""Read-only, two-provider NEAR testnet RPC consistency, NEVER checkpoint trust.

Collect stores source URL, JSON-RPC method/params, raw response SHA256 and
minimal protocol/header fields. Verify is entirely offline and fails closed.
Neither provider agreement nor a local digest provides consensus authority.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import re
from datetime import datetime, timezone
from pathlib import Path
from urllib.request import Request, urlopen

from near_authenticated import checkpoint_checksum

SCHEMA = 'x3.near-checkpoint-rpc-consistency.v1'
RPCS = ('https://rpc.testnet.near.org', 'https://test.rpc.fastnear.com')
REQUESTS = {
    'status': ('status', []),
    'genesis_config': ('EXPERIMENTAL_genesis_config', []),
    'historical_protocol': ('EXPERIMENTAL_protocol_config', {'block_id': 276392452}),
    'previous_block': ('block', {'block_id': 276349252}),
    'candidate_block': ('block', {'block_id': 276392452}),
}

class CrosscheckError(ValueError):
    pass


def sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def canon(obj: object) -> bytes:
    return json.dumps(obj, sort_keys=True, separators=(',', ':'), ensure_ascii=False, allow_nan=False).encode()


def summary(kind: str, data: dict) -> dict:
    if kind == 'status':
        return {key: data.get(key) for key in ('chain_id', 'genesis_hash')}
    if kind in ('genesis_config', 'historical_protocol'):
        return {key: data.get(key) for key in ('chain_id', 'protocol_version', 'genesis_height', 'genesis_time', 'epoch_length')}
    if kind in ('previous_block', 'candidate_block'):
        header = data.get('header')
        if not isinstance(header, dict):
            raise CrosscheckError('missing block header')
        return {key: header.get(key) for key in ('height', 'hash', 'epoch_id', 'next_epoch_id', 'latest_protocol_version')}
    raise CrosscheckError('unexpected request kind')


def collect(root: Path, *, timeout: int = 12) -> dict:
    records = []
    for url in RPCS:
        calls = {}
        for kind, (method, params) in REQUESTS.items():
            payload = {'jsonrpc': '2.0', 'id': 'x3-read-only-checkpoint', 'method': method, 'params': params}
            req = Request(url, data=canon(payload), headers={'Content-Type':'application/json','User-Agent':'X3-ReadOnly-Checkpoint/1.0'})
            with urlopen(req, timeout=timeout) as response:
                raw = response.read(2_000_001)
            if len(raw) > 2_000_000:
                raise CrosscheckError('oversized RPC response')
            rpc = json.loads(raw)
            if rpc.get('error') is not None or not isinstance(rpc.get('result'), dict):
                raise CrosscheckError(f'{url}: {kind} RPC failure: {rpc.get("error")}')
            calls[kind] = {
                'method': method, 'params': params,
                'request_sha256': sha(canon(payload)),
                'raw_response_sha256': sha(raw),
                'summary': summary(kind, rpc['result']),
            }
        records.append({'url': url, 'calls': calls})
    body = {
        'schema': SCHEMA,
        'recorded_at_utc': datetime.now(timezone.utc).isoformat(timespec='seconds'),
        'status': 'READ_ONLY_RPC_AGREEMENT_NOT_AUTHENTICATED',
        'authenticated': False, 'authorized_to_sign': False, 'checkpoint_approved': False,
        'providers': records,
    }
    body['integrity_sha256'] = sha(canon(body))
    verify_document(body, root)
    return body


def verify_document(body: dict, root: Path) -> dict:
    if body.get('schema') != SCHEMA or body.get('status') != 'READ_ONLY_RPC_AGREEMENT_NOT_AUTHENTICATED':
        raise CrosscheckError('invalid source/authorization schema')
    if any(body.get(flag) is not False for flag in ('authenticated','authorized_to_sign','checkpoint_approved')):
        raise CrosscheckError('RPC evidence cannot claim signing or authentication')
    without_digest = {k:v for k,v in body.items() if k != 'integrity_sha256'}
    if body.get('integrity_sha256') != sha(canon(without_digest)):
        raise CrosscheckError('RPC crosscheck report checksum mismatch')
    providers = body.get('providers')
    if not isinstance(providers,list) or len(providers)!=2 or [x.get('url') for x in providers] != list(RPCS):
        raise CrosscheckError('expected two named, distinct testnet RPC services')
    checkpoint = json.loads((root / 'checkpoint.unsigned.json').read_text())
    previous = json.loads((root / 'previous-epoch-rpc.json').read_text())['result']['inner_lite']
    if checkpoint_checksum(checkpoint) != checkpoint.get('checksum'):
        raise CrosscheckError('unsigned checkpoint checksum mismatch')
    first = providers[0]['calls']
    for provider in providers:
        calls = provider.get('calls')
        if not isinstance(calls,dict) or set(calls) != set(REQUESTS):
            raise CrosscheckError('missing RPC method evidence')
        for name, (method,params) in REQUESTS.items():
            record = calls[name]
            expected_req = {'jsonrpc':'2.0','id':'x3-read-only-checkpoint','method':method,'params':params}
            if record.get('method') != method or record.get('params') != params or record.get('request_sha256') != sha(canon(expected_req)):
                raise CrosscheckError('RPC request/provenance mismatch')
            if not isinstance(record.get('raw_response_sha256'),str) or re.fullmatch('[a-f0-9]{64}',record['raw_response_sha256']) is None:
                raise CrosscheckError('missing RPC raw response digest')
            if record.get('summary') != first[name].get('summary'):
                raise CrosscheckError('RPC providers disagree about a checkpoint field')
    st=first['status']['summary']
    genesis=first['genesis_config']['summary']
    proto=first['historical_protocol']['summary']
    old=first['previous_block']['summary']
    new=first['candidate_block']['summary']
    if st != {'chain_id':'testnet','genesis_hash':checkpoint.get('genesis_hash')}:
        raise CrosscheckError('testnet chain identity or genesis mismatch')
    if genesis.get('chain_id') != 'testnet' or proto.get('chain_id') != 'testnet' or proto.get('protocol_version') != checkpoint.get('protocol_version'):
        raise CrosscheckError('historical protocol identity/version mismatch')
    if genesis.get('protocol_version') != 29 or genesis.get('genesis_height') != 42376888 or genesis.get('epoch_length') != 43200:
        raise CrosscheckError('unexpected public testnet genesis metadata')
    for key in ('genesis_height','genesis_time','epoch_length'):
        if proto.get(key) != genesis.get(key):
            raise CrosscheckError('inconsistent historical protocol/genesis field')
    head = checkpoint['light_client_block']['inner_lite']
    if new != { 'height':checkpoint['block_height'], 'hash':checkpoint['block_hash'],
        'epoch_id':checkpoint['epoch_id'],'next_epoch_id':checkpoint['next_epoch_id'],
        'latest_protocol_version':checkpoint['protocol_version'] }:
        raise CrosscheckError('historic candidate block mismatch')
    if old != {'height':previous['height'], 'hash':'FXA5BRLmqPP1uPfFx6Ndd5cyLZnHTvjpZ2oBEgoxxYpP',
        'epoch_id':previous['epoch_id'], 'next_epoch_id':previous['next_epoch_id'],
        'latest_protocol_version':checkpoint['protocol_version']}:
        raise CrosscheckError('historic predecessor block mismatch')
    if new['epoch_id'] != old['next_epoch_id'] or new['height'] <= old['height'] or head['epoch_id'] != new['epoch_id']:
        raise CrosscheckError('inconsistent historical epoch progression')
    return {'rpc_providers_consistent': True, 'recorded_at_utc': body['recorded_at_utc'],
            'checkpoint_height':new['height'], 'historical_protocol_version':proto['protocol_version'],
            'authenticated':False, 'authorized_to_sign':False}


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('action', choices=('collect','verify'))
    parser.add_argument('--root',type=Path,required=True)
    parser.add_argument('--report',type=Path,required=True)
    args=parser.parse_args()
    if args.action=='collect':
        document=collect(args.root)
        with args.report.open('xb') as stream:
            stream.write(json.dumps(document, indent=2, sort_keys=True, ensure_ascii=False).encode()+b'\n')
    else:
        document=json.loads(args.report.read_text())
    print(json.dumps(verify_document(document,args.root),indent=2,sort_keys=True))

if __name__=='__main__':
    main()
