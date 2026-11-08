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
from datetime import datetime
from hashlib import sha256
import json
from pathlib import Path
import sys

from vault_origin_base_proof import Expected, ProofRejected, _https_rpc,verify_two_providers as verify_base
from near_delivery_receipt_proof import NearExpected,NearProofRejected,_https_near_rpc,verify_two_providers as verify_near
from vault_two_chain_pairing import correlate


def _strict_object(doc:dict,fields_:set[str],extra:set[str],reason:str)->None:
    if not isinstance(doc,dict) or set(doc)!=fields_|extra:
        raise ProofRejected(reason)


def execute(argv:list[str]|None=None)->int:
    p=argparse.ArgumentParser(description='Read-only two-chain VAULT-origin proof. Requires new real transaction receipts.')
    p.add_argument('--base-manifest',required=True,type=Path)
    p.add_argument('--near-manifest',required=True,type=Path)
    p.add_argument('--signed-quote-file',required=True,type=Path)
    p.add_argument('--output',required=True,type=Path)
    args=p.parse_args(argv)
    try:
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
            from base_sepolia_x3_intent import compile_testnet_intent
            import near_guard_quorum
        except ImportError as exc:
            raise ProofRejected('CANONICAL_X3_GUARD_AND_MAKER_VERIFIERS_MISSING') from exc
        if not isinstance(response.get('timestamp'),str):raise ProofRejected('QUOTE_TIMESTAMP_MISSING')
        issued=datetime.fromisoformat(response['timestamp'].replace('Z','+00:00'))
        if issued.utcoffset() is None:raise ProofRejected('QUOTE_TIMESTAMP_MUST_BE_UTC')
        q=admission.check_quote(response,issued) # Pinned authentic Ed25519 signature and terms
        source=compile_testnet_intent() # Real .x3 frontend; must be fresh new nonce
        if q['quote_sha256']!=ebase.quote_sha256 or q['quote_sha256']!=enear.quote_sha256:
            raise ProofRejected('SIGNED_QUOTE_HASH_NOT_BOUND_TO_VAULT_AND_GUARD')
        if source['program_sha256']!=ebase.program_sha256 or source['program_sha256']!=enear.program_sha256:
            raise ProofRejected('X3_COMPILED_PROGRAM_HASH_NOT_BOUND_TO_VAULT_AND_GUARD')
        if source['nonce']!=enear.nonce:
            raise ProofRejected('OWNER_X3_COMPILED_NONCE_NOT_MATCHED')
        # The first finalized historical Tidepool trade already used this
        # specific owner nonce with a direct EOA source transaction. Never
        # reuse it as the authorization of a NEW contract-vault trade.
        if source['nonce']=='x3_base_testnet_trade_001':
            raise ProofRejected('HISTORICAL_ALREADY_USED_X3_NONCE_FOR_NEW_VAULT')
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
        if q['minimum_out']!=enear.minimum_units:
            raise ProofRejected('MAKER_SIGNED_MINIMUM_NOT_PINNED')
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
        if not int(issued.timestamp())-120<=included<=int(q['deadline'].timestamp()):
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
