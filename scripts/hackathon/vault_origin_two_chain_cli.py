#!/usr/bin/env python3
"""Owner-reviewed, READ-ONLY Base vault and NEAR settlement evidence candidate.

No signing, deployment, asset transfer, wallet access, or public chain writes.
A real proof can only exist AFTER explicitly owner-approved vault deployment,
funding, signing, settlement, and finalized chain transactions. The current
historical EOA transfer must fail at the Base vault-origin gate.

Requires existing X3Lang modules after this patch is copied to the repo:
base_sepolia_guarded_admission (strict real Tidepool maker signature),
base_sepolia_x3_intent (real frontend parser/typecheck),
near_guard_quorum (reviewed canonical Guard identity pin).
"""
from __future__ import annotations

import argparse
from dataclasses import fields
from hashlib import sha256
import json
import re
from pathlib import Path
import sys

from vault_origin_base_proof import Expected, ProofRejected, _https_rpc,verify_two_providers as verify_base
from near_delivery_receipt_proof import NearExpected,NearProofRejected,_https_near_rpc,verify_two_providers as verify_near
from vault_two_chain_pairing import correlate

REPO_ROOT = Path(__file__).resolve().parents[2]
LEGACY_SPENT_NONCE = 'x3_base_testnet_trade_001'


def verified_fresh_x3_source(source_path: Path, nonce: str, expected_sha256: str) -> dict:
    """Reparse owner-selected .x3 source; refuse legacy and unpinned source changes."""
    if not isinstance(nonce,str) or re.fullmatch(r'x3_[a-z0-9_]{5,61}',nonce) is None:
        raise ProofRejected('NEW_X3_OWNER_NONCE_INVALID')
    if nonce == LEGACY_SPENT_NONCE:
        raise ProofRejected('HISTORICAL_ALREADY_USED_X3_NONCE_FOR_NEW_VAULT')
    if not isinstance(expected_sha256,str) or re.fullmatch(r'[0-9a-f]{64}',expected_sha256) is None:
        raise ProofRejected('OWNER_REVIEWED_X3_PROGRAM_HASH_REQUIRED')
    if not isinstance(source_path, Path):
        raise ProofRejected('APPROVED_X3_SOURCE_PATH_REQUIRED')
    path = source_path if source_path.is_absolute() else REPO_ROOT/source_path
    resolved = path.resolve()
    if not resolved.is_relative_to(REPO_ROOT) or path.is_symlink() or not path.is_file():
        raise ProofRejected('X3_SOURCE_MUST_BE_EXISTING_REPO_FILE_NO_SYMLINK')
    try:
        from base_sepolia_x3_intent import compile_testnet_intent
        compiled = compile_testnet_intent(path, expected_nonce=nonce)
    except (ImportError, OSError, ValueError) as exc:
        raise ProofRejected('X3_CANDIDATE_SOURCE_NOT_VERIFIED_BY_REAL_FRONTEND') from exc
    if compiled.get('program_sha256') != expected_sha256 or compiled.get('nonce') != nonce:
        raise ProofRejected('X3_CANDIDATE_SOURCE_HASH_OR_NONCE_DRIFT')
    return compiled


def _strict_object(doc:dict,fields_:set[str],extra:set[str],reason:str)->None:
    if not isinstance(doc,dict) or set(doc)!=fields_|extra:
        raise ProofRejected(reason)


def execute(argv:list[str]|None=None)->int:
    p=argparse.ArgumentParser(description='Read-only two-chain VAULT-origin proof. Requires new real transaction receipts.')
    p.add_argument('--base-manifest',required=True,type=Path)
    p.add_argument('--near-manifest',required=True,type=Path)
    p.add_argument('--signed-quote-file',required=True,type=Path)
    p.add_argument('--output',required=True,type=Path)
    p.add_argument('--fresh-x3-source',type=Path)
    p.add_argument('--expected-owner-nonce')
    p.add_argument('--expected-program-sha256')
    args=p.parse_args(argv)
    try:
        if not (args.fresh_x3_source and args.expected_owner_nonce and args.expected_program_sha256):
            raise ProofRejected('FRESH_X3_SOURCE_NONCE_AND_SHA_REQUIRED')
        base=json.loads(args.base_manifest.read_text())
        near=json.loads(args.near_manifest.read_text())
        raw_quote=json.loads(args.signed_quote_file.read_text())
        _strict_object(base,{x.name for x in fields(Expected)},{'tx_hash','rpc_urls'},'BASE_MANIFEST_FIELD_MISMATCH')
        _strict_object(near,{x.name for x in fields(NearExpected)},{'rpc_urls'},'NEAR_MANIFEST_FIELD_MISMATCH')
        if not isinstance(raw_quote,dict) or raw_quote.get('classification')!='LIVE_QUOTE_UNFUNDED_UNAUTHORIZED_NOT_SETTLED':
            raise ProofRejected('ORIGINAL_SIGNED_TIDEPOOL_QUOTE_REQUIRED')
        response=raw_quote.get('response')
        if not isinstance(response,dict):raise ProofRejected('MISSING_SIGNED_MAKER_QUOTE')
        ebase=Expected(**{x.name:base[x.name] for x in fields(Expected)})
        enear=NearExpected(**{x.name:near[x.name] for x in fields(NearExpected)})
        ebase.validate();enear.validate()
        if (not isinstance(base['rpc_urls'],list) or len(base['rpc_urls'])!=2
                or not isinstance(near['rpc_urls'],list) or len(near['rpc_urls'])!=2):
            raise ProofRejected('TWO_INDEPENDENT_RPCS_PER_CHAIN_REQUIRED')
        # Real canonical quote/Guard modules are dependencies from X3Lang; no
        # untrusted manifest boolean can substitute for these verifiers.
        try:
            import base_sepolia_guarded_admission as admission
            import near_guard_quorum
        except ImportError as exc:
            raise ProofRejected('CANONICAL_X3_GUARD_AND_MAKER_VERIFIERS_MISSING') from exc
        source=verified_fresh_x3_source(args.fresh_x3_source,args.expected_owner_nonce,args.expected_program_sha256)
        try:
            q=admission.verify_signed_quote(response)
        except admission.AdmissionError as exc:
            raise ProofRejected(str(exc)) from exc
        if not 60 <= (q['deadline']-q['issued_at']).total_seconds() <= source['timeout_minutes']*60:
            raise ProofRejected('X3_COMPILED_TIMEOUT_EXCEEDED')
        if q['minimum_out'] < source['minimum_out']:
            raise ProofRejected('X3_COMPILED_MIN_RECEIVE_VIOLATED')
        if q['quote_sha256']!=ebase.quote_sha256 or q['quote_sha256']!=enear.quote_sha256:
            raise ProofRejected('SIGNED_QUOTE_HASH_NOT_BOUND_TO_VAULT_AND_GUARD')
        if source['program_sha256']!=ebase.program_sha256 or source['program_sha256']!=enear.program_sha256:
            raise ProofRejected('X3_COMPILED_PROGRAM_HASH_NOT_BOUND_TO_VAULT_AND_GUARD')
        if source['nonce']!=enear.nonce:
            raise ProofRejected('OWNER_X3_COMPILED_NONCE_NOT_MATCHED')
        # Neither a synthetic quote nor arbitrary review-manifest fields may
        # override the real X3 canonical testnet route and pinned token.
        if (ebase.token.lower()!=admission.TOKEN.lower()
                or ebase.token_runtime_sha256!=admission.TOKEN_CODE_SHA256):
            raise ProofRejected('UNAPPROVED_BASE_TEST_TOKEN_OR_CODE')
        if ebase.owner.lower()!=source['source_account'].lower():
            raise ProofRejected('SOURCE_VAULT_OWNER_NOT_COMPILED_X3_OWNER')
        if ebase.amount!=admission.AMOUNT or enear.origin_amount!=admission.AMOUNT:
            raise ProofRejected('UNAPPROVED_BASE_SOURCE_AMOUNT')
        if (ebase.max_per_payment!=admission.AMOUNT or ebase.max_total!=admission.AMOUNT):
            raise ProofRejected('WRONG_OWNER_REVIEWED_VAULT_BUDGET')
        if (enear.recipient!=admission.RECEIVER or
                enear.minimum_units!=q['minimum_out']):
            raise ProofRejected('WRONG_SIGNED_NEAR_DESTINATION_OR_MINIMUM')
        if (ebase.agent.lower()==ebase.owner.lower() or
                ebase.agent.lower()==ebase.deposit_address.lower()):
            raise ProofRejected('AGENT_KEY_NOT_SEPARATE_FROM_FUNDED_OWNER')
        if q['deposit_address'].lower()!=ebase.deposit_address.lower():
            raise ProofRejected('MAKER_SIGNED_DEPOSIT_ADDRESS_NOT_VAULT_RECIPIENT')
        if source['refund_to'].lower()!=response['quoteRequest']['refundTo'].lower():
            raise ProofRejected('MAKER_REFUND_TO_NOT_OWNER_REVIEWED')
        if (enear.refund_to.lower()!=source['refund_to'].lower()
                or enear.compiled_minimum_units!=source['minimum_out']
                or enear.slippage_max_bps!=source['slippage_max_bps']
                or enear.quote_deadline_unix_s!=int(q['deadline'].timestamp())):
            raise ProofRejected('HISTORICAL_GUARD_POLICY_NOT_BOUND_TO_COMPILED_X3_QUOTE')
        near_guard_quorum.validate_guard_identity(enear.guard_account,enear.guard_code_sha256)
        # Real source facts first; historical EOA ERC20 transfer is instantly denied.
        b=verify_base(((base['rpc_urls'][0],_https_rpc(base['rpc_urls'][0])),
                       (base['rpc_urls'][1],_https_rpc(base['rpc_urls'][1]))),ebase,base['tx_hash'])
        included=b['observations'][0]['inclusion_unix_s']
        if not int(q['issued_at'].timestamp())<=included<=int(q['deadline'].timestamp()):
            raise ProofRejected('BASE_VAULT_PAYMENT_OUTSIDE_MAKER_SIGNED_QUOTE_WINDOW')
        if included!=enear.source_inclusion_unix_s:
            raise ProofRejected('NEAR_RECEIPT_INPUT_SOURCE_TIME_MISMATCH')
        n=verify_near(((near['rpc_urls'][0],_https_near_rpc(near['rpc_urls'][0])),
                       (near['rpc_urls'][1],_https_near_rpc(near['rpc_urls'][1]))),enear)
        result=correlate(b,n,ebase,enear)
        result['schema']='x3.shield.vault_origin_two_chain_readonly_v1'
        result['maker_signature_verified_by_trusted_x3lang_module']=True
        result['maker_account']='maker.tidepool-sandbox.testnet'
        result['guard_historical_approval_classification']='TWO_NEAR_FINALIZED_RPC_OBSERVATIONS_NOT_VALIDATOR_SIGNATURE_PROOF'
        result['owner_manifest_is_cryptographically_signed']=False
        result['provider_is_third_party_custodial_relayer']=True
        result['cross_chain_atomicity_proven']=False
        result['base_evidence']=b
        result['near_evidence']=n
        # Cryptographic hashes ensure artifact integrity, not truthfulness or
        # provider independence. Create once and never overwrite evidence.
        serialized=(json.dumps(result,sort_keys=True,indent=2)+'\n').encode()
        args.output.parent.mkdir(parents=True,exist_ok=True)
        with args.output.open('xb') as f:f.write(serialized)
        print(json.dumps({'classification':result['classification'],'evidence_sha256':sha256(serialized).hexdigest(),
                          'path':str(args.output),'signing_or_funding_performed':False,
                          'trustless_atomicity_proven':False},sort_keys=True))
        return 0
    except (ProofRejected,NearProofRejected,ValueError,TypeError,KeyError,OSError,OverflowError) as exc:
        print(json.dumps({'classification':'HARD_DENIED_NO_TWO_CHAIN_VAULT_PROOF','reason':str(exc),
                          'signed':False,'funded':False,'broadcast':False},sort_keys=True),file=sys.stderr)
        return 2

if __name__=='__main__':raise SystemExit(execute())
