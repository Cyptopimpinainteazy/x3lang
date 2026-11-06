#!/usr/bin/env python3
"""Offline X3Lang judge demo: real bytecode VM + genuine archived NEAR proofs.

NO settlement permission: VM execution is local/dry; real receipt proof is
for an unrelated NEAR contract and is checked against an UNTRUSTED RPC head.
The VM and NEAR proof verifier are separate demonstrators, not an executable
cross-chain authorization integration.
"""
from __future__ import annotations

import argparse
import copy
import hashlib
import json
import os
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
BRIDGE = ROOT / 'x3-lang' / 'bridge'
if str(BRIDGE) not in sys.path:
    sys.path.insert(0, str(BRIDGE))

from near_outcome import verify_receipt_outcome  # noqa: E402
from near_merkle import decode_hash  # noqa: E402
from near_validator_commitment import check_next_bps_commitment  # noqa: E402
from checkpoint_provider_crosscheck import verify_document  # noqa: E402

SCHEMA = 'x3.judge.crypto_demo.v1'
SOURCE = 'x3-lang/examples/timeout_refund_minimal.x3'
RECEIPT = 'x3-lang/bridge/fixtures/near_live_receipt_proof_20261105.json'
RECEIPT_SHA = '0fb16971ff172bd30e6ada514a3c9ea688ac77815ea506f28be09b6b3512d5ab'
UPDATE = 'x3-lang/bridge/fixtures/near_live_light_client_update_20261105.json'
UPDATE_SHA = 'c47f0c381dbd9a37ac432aa4f77cf9d47ea8ccc452bfe4ff2a67b868953caed7'
CHECKPOINT_REVIEW_ROOT = 'evidence/near-checkpoint-review/p27'
CROSS_REPORT = 'two-provider-crosscheck.json'
RECEIPT_ID = '3f9FCzqC8k75AGob2UrCxbKVqADRyv5M5vGfoLEXbZyu'
RECEIVER = 'league-v7.near-post.testnet'
HEAD = 'V7KcHcg16BVWWxhVbw2vQj87D3dJQwXchnU7DCVzo6V'
# Historical `block` RPC observation, NOT independently authenticated finality.
UNTRUSTED_HEAD_MERKLE_ROOT = '6F2mBDUkzB8tRpETPpRTtH5zo1E5yGGpftfyNYbRVDVw'


class DemoError(RuntimeError):
    pass


def sha256(raw: bytes) -> str:
    return hashlib.sha256(raw).hexdigest()


def validate_pinned_fixture(path: Path, expected_sha256: str) -> dict:
    raw = path.read_bytes()
    if sha256(raw) != expected_sha256:
        raise DemoError(f'pinned genuine NEAR RPC fixture has changed: {path.name}')
    def reject_duplicate(pairs):
        result = {}
        for key, value in pairs:
            if key in result:
                raise DemoError('duplicate field in NEAR evidence')
            result[key] = value
        return result
    try:
        document = json.loads(raw, object_pairs_hook=reject_duplicate,
                              parse_constant=lambda value: (_ for _ in ()).throw(DemoError('invalid JSON constant')))
    except (UnicodeError, ValueError) as exc:
        raise DemoError('invalid archived NEAR evidence') from exc
    if not isinstance(document, dict) or document.get('error') is not None or document.get('jsonrpc') != '2.0':
        raise DemoError('invalid NEAR RPC response envelope')
    return document


def run_x3c(x3c: Path, args: list[str], cwd: Path) -> str:
    try:
        result = subprocess.run([str(x3c.resolve()), *args], cwd=cwd, capture_output=True,
                                text=True, timeout=120, check=False)
    except (OSError, subprocess.TimeoutExpired) as exc:
        raise DemoError('production x3c binary failed to execute') from exc
    if result.returncode != 0:
        raise DemoError(f'x3c {args[0]} failed (exit {result.returncode}): {(result.stdout+result.stderr)[-1000:]}')
    return result.stdout.strip()


def _check_tampering(proof: dict, update: dict) -> list[dict]:
    cases = []
    mutations = [
        ('modified_receipt_gas', lambda x: x['outcome_proof']['outcome'].__setitem__('gas_burnt', 1)),
        ('modified_executor', lambda x: x['outcome_proof']['outcome'].__setitem__('executor_id', 'attacker.testnet')),
        ('reversed_block_merkle_branch', lambda x: x['block_proof'][0].__setitem__(
            'direction', 'Left' if x['block_proof'][0]['direction'] == 'Right' else 'Right')),
    ]
    for name, mutate in mutations:
        altered = copy.deepcopy(proof)
        mutate(altered['result'])
        try:
            verify_receipt_outcome(altered, receipt_id=RECEIPT_ID, receiver_id=RECEIVER,
                                   light_client_head=HEAD,
                                   head_block_merkle_root=decode_hash(UNTRUSTED_HEAD_MERKLE_ROOT))
        except ValueError as exc:
            cases.append({'attack': name, 'rejected': True, 'rejection_type': type(exc).__name__})
        else:
            raise DemoError('real receipt tampering unexpectedly accepted: ' + name)
    altered = copy.deepcopy(update['result'])
    altered['next_bps'][0]['stake'] = '1'
    try:
        check_next_bps_commitment(altered)
    except ValueError as exc:
        cases.append({'attack':'modified_validator_stake', 'rejected':True,'rejection_type':type(exc).__name__})
    else:
        raise DemoError('modified validator commitment unexpectedly accepted')
    return cases


def _vm_counts(stdout: str) -> dict[str, int]:
    if not stdout.startswith('x3c run: ok'):
        raise DemoError('VM did not positively report local execution success')
    match = re.search(r'(?P<assets>\d+) asset ops, (?P<bridges>\d+) bridge ops, (?P<receipts>\d+) receipts, gas remaining (?P<gas>\d+)', stdout)
    if not match:
        raise DemoError('unrecognized VM execution report')
    counts = {k:int(v) for k,v in match.groupdict().items()}
    if counts['bridges'] < 1 or counts['receipts'] < 1:
        raise DemoError('expected local VM route/receipt did not execute')
    return counts


def build_report(*, root: Path = ROOT, x3c: Path, output_dir: Path) -> dict:
    root = root.resolve()
    if not x3c.is_file() or not os.access(x3c, os.X_OK):
        raise DemoError('production Rust x3c executable missing: build x3-tools --bin x3c first')
    source = root / SOURCE
    if not source.is_file():
        raise DemoError('missing verified real X3Lang source')
    output_dir.mkdir(parents=True, exist_ok=True)
    artifact = output_dir / 'timeout_refund_minimal.x3b'
    check_text = run_x3c(x3c, ['check', str(source)], root)
    if '"status": "ok"' not in check_text:
        raise DemoError('x3c check did not return a verified successful status')
    build_text = run_x3c(x3c, ['build', str(source), '--out', str(artifact)], root)
    if not artifact.is_file() or artifact.stat().st_size == 0:
        raise DemoError('real compiler did not create bytecode')
    run_text = run_x3c(x3c, ['run', str(artifact)], root)
    counts = _vm_counts(run_text)

    proof = validate_pinned_fixture(root / RECEIPT, RECEIPT_SHA)
    update = validate_pinned_fixture(root / UPDATE, UPDATE_SHA)
    checked_receipt = verify_receipt_outcome(
        proof, receipt_id=RECEIPT_ID, receiver_id=RECEIVER,
        light_client_head=HEAD, head_block_merkle_root=decode_hash(UNTRUSTED_HEAD_MERKLE_ROOT))
    if not (checked_receipt.outcome_merkle_matches and checked_receipt.block_merkle_matches):
        raise DemoError('actual NEAR receipt Merkle proof failed')
    if checked_receipt.authenticated or checked_receipt.authorized_to_sign:
        raise DemoError('unexpected trusted result for untrusted RPC-provided root')
    commitment = check_next_bps_commitment(update['result'])
    if commitment.authenticated or commitment.authorized_to_sign:
        raise DemoError('untrusted validator commitment improperly authenticated')
    review_root = root / CHECKPOINT_REVIEW_ROOT
    crosscheck = verify_document(json.loads((review_root / CROSS_REPORT).read_text()), review_root)
    if crosscheck['authenticated'] or crosscheck['authorized_to_sign']:
        raise DemoError('provider agreement improperly authenticated')
    attacks = _check_tampering(proof, update)
    git_commit = subprocess.run(['git', 'rev-parse', 'HEAD'], cwd=root, text=True,
                                capture_output=True, check=True, timeout=10).stdout.strip()
    return {
        'schema':SCHEMA,
        'execution_scope':'offline_read_only; no contracts deployed; no assets transferred',
        'repository_commit':git_commit,
        'compiler_vm':{
            'driver':'real Rust x3-tools x3c binary',
            'source':SOURCE,'source_sha256':sha256(source.read_bytes()),
            'check':{'status':'passed','output':check_text},
            'build':{'status':'passed','output':build_text},
            'artifact_bytes':artifact.stat().st_size,
            'artifact_sha256':sha256(artifact.read_bytes()),
            'execution':{'status':'passed','mode':'local_dry_run','output':run_text,**counts},
        },
        'near':{
            'receipt':{
                'provenance':'genuine archived NEAR testnet RPC response from PR #57; NOT an X3 escrow',
                'file':RECEIPT,'source_sha256':RECEIPT_SHA,
                'receipt_id':RECEIPT_ID,'receiver_id':RECEIVER,
                'block_hash':checked_receipt.block_hash,
                'shard_outcome_root':checked_receipt.shard_outcome_root,
                'block_outcome_root':checked_receipt.block_outcome_root,
                'light_client_head':HEAD,
                'head_root':UNTRUSTED_HEAD_MERKLE_ROOT,
                'head_root_authority':'UNTRUSTED historical public RPC response',
                'outcome_merkle_matches':checked_receipt.outcome_merkle_matches,
                'block_merkle_matches':checked_receipt.block_merkle_matches,
                'authenticated':False,'authorized_to_sign':False,
            },
            'validator_commitment':{
                'file':UPDATE,'source_sha256':UPDATE_SHA,
                'validator_count':commitment.validator_count,
                'next_bp_hash':commitment.next_bp_hash,'matches':True,
                'authenticated':False,'authorized_to_sign':False,
            },
            'cross_provider':{
                'report':CHECKPOINT_REVIEW_ROOT+'/'+CROSS_REPORT,
                'consistent':crosscheck['rpc_providers_consistent'],
                'authenticated':False,'authorized_to_sign':False,
            },
        },
        'adversarial_tests':attacks,
        'release_authority':{
            'trusted_checkpoint':False,
            'escrow_storage_authenticated':False,
            'cross_chain_causality_proven':False,
            'authorized_to_sign':False,
            'funds_moved':False,
        },
        'limitations':[
            'Compiler and local VM run are genuine but use dry-run execution with no network, bridge execution, or wallet authorization.',
            'Rust VM policy demonstration and Python NEAR proof verification are separate components, not a connected proof hostcall.',
            'Receipt proof is genuine historical testnet data from an unrelated contract, not the X3 vault.',
            'Merkle inclusion is relative to an untrusted head root from historical public RPC; no independently approved checkpoint exists.',
            'Validator-list commitment and two-RPC agreement do not establish independently trusted consensus finality.',
            'Escrow storage, code identity, terminal claim/refund, cross-chain atomicity and certificate signing remain unverified.',
            'A separate NEAR Intents .x3 agent example currently uses under-mandate syntax not supported by Rust x3c; Python policy preview is separate.',
        ],
    }


def write_report(report: dict, path: Path):
    tmp = path.with_name(path.name + '.tmp')
    try:
        with tmp.open('x', encoding='utf-8') as stream:
            json.dump(report, stream, indent=2, sort_keys=True)
            stream.write('\n')
        tmp.replace(path)
    except BaseException:
        tmp.unlink(missing_ok=True)
        raise


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output-dir',type=Path,default=Path('/tmp/x3lang-judge-crypto-demo'))
    parser.add_argument('--x3c',type=Path,default=ROOT/'x3-lang/target/debug/x3c')
    args=parser.parse_args(argv)
    try:
        result=build_report(root=ROOT, x3c=args.x3c, output_dir=args.output_dir)
        write_report(result,args.output_dir/'report.json')
    except (DemoError, ValueError, OSError, KeyError, json.JSONDecodeError, subprocess.SubprocessError) as exc:
        print('X3LANG CRYPTOGRAPHIC DEMO BLOCKED: '+str(exc),file=sys.stderr)
        return 1
    print('X3Lang real compiler/VM dry-run: PASS | source='+result['compiler_vm']['source'])
    print('Bytecode SHA256: '+result['compiler_vm']['artifact_sha256'])
    print('Real archived NEAR outcome + block Merkle proof: MATCH (root NOT authenticated)')
    print('Real NEAR validator-list commitment: MATCH (validator set NOT authenticated)')
    print('Two provider metadata observations: CONSISTENT (NOT independently trusted)')
    print(f'Adversarial rejected: {len(result["adversarial_tests"])} / {len(result["adversarial_tests"])}')
    print('TRUSTED CHECKPOINT: FALSE | AUTHORIZED TO SIGN: FALSE | FUNDS MOVED: FALSE')
    print('Machine-readable report: '+str(args.output_dir/'report.json'))
    return 0

if __name__=='__main__':
    raise SystemExit(main())
