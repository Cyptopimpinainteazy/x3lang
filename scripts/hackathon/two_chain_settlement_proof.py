#!/usr/bin/env python3
"""X3Lang independent two-chain settlement evidence; TESTNET ONLY, NO SIGNING.

This reporter makes no cross-chain atomicity claim: it checks transaction facts
from two independent Base Sepolia RPC providers, NEAR finalized state and a
NEAR USDC transfer receipt. A maker status response alone is never evidence.
"""
from __future__ import annotations

import argparse
import base64
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import sys
from typing import Any

from base_sepolia_guarded_admission import (
    NEAR_RPC, NEAR_GENESIS, BASE_RPCS, CHAIN_ID, SOURCE, TOKEN, TOKEN_CODE_SHA256,
    RECEIVER, AMOUNT, DEST_ASSET, AdmissionError, reject, rpc, check_quote,
    get_near_policy, check_guard_policy, enforce_compiled_x3_policy,
)

USDC_NEAR_CONTRACT = 'usdc.tidepool-sandbox.testnet'
ERC20_TRANSFER_TOPIC = '0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef'
MIN_BASE_CONFIRMATIONS = 12
MAX_BLOCK_DRIFT = 6  # finalized snapshots should be near each other across providers


def is_tx_hash(value: str) -> bool:
    return isinstance(value,str) and len(value)==66 and value.startswith('0x') and all(x in '0123456789abcdefABCDEF' for x in value[2:])


def neartime(block: dict) -> dict:
    return {'block_hash':block.get('block_hash'),'block_height':block.get('block_height')}


def near_ft_balance(account: str, block_id: str | None = None) -> dict:
    args=base64.b64encode(json.dumps({'account_id':account},separators=(',',':')).encode()).decode()
    p={'request_type':'call_function','account_id':USDC_NEAR_CONTRACT,'method_name':'ft_balance_of','args_base64':args}
    if block_id:p['block_id']=block_id
    else:p['finality']='final'
    reply=rpc(NEAR_RPC,'query',p)
    raw=json.loads(bytes(reply['result']).decode())
    reject(isinstance(raw,str) and raw.isdecimal(),'NEAR_FT_BALANCE_MALFORMED')
    reject(isinstance(reply.get('block_hash'),str) and isinstance(reply.get('block_height'),int),'NEAR_FINAL_BLOCK_MISSING')
    return {'units':int(raw),**neartime(reply)}


def require_base_tx(rpc_url: str, txid: str, destination: str) -> dict:
    reject(is_tx_hash(txid),'INVALID_BASE_TX_HASH')
    reject(int(rpc(rpc_url,'eth_chainId',[]),16)==CHAIN_ID,'BASE_CHAIN_ID_MISMATCH')
    tx=rpc(rpc_url,'eth_getTransactionByHash',[txid])
    receipt=rpc(rpc_url,'eth_getTransactionReceipt',[txid])
    reject(isinstance(tx,dict) and isinstance(receipt,dict),'BASE_TX_OR_RECEIPT_NOT_FINAL')
    reject(tx.get('hash','').lower()==txid.lower() and receipt.get('transactionHash','').lower()==txid.lower(),'BASE_TX_HASH_MISMATCH')
    reject(tx.get('from','').lower()==SOURCE.lower() and tx.get('to','').lower()==TOKEN.lower(),'BASE_SENDER_OR_CONTRACT_MISMATCH')
    reject(int(tx.get('value','0x0'),16)==0,'BASE_UNEXPECTED_NATIVE_VALUE')
    calldata=tx.get('input','')
    expected='0xa9059cbb'+destination[2:].lower().zfill(64)+format(AMOUNT,'064x')
    reject(calldata.lower()==expected.lower(),'BASE_TRANSFER_CALLDATA_MISMATCH')
    reject(int(receipt.get('status','0x0'),16)==1,'BASE_TX_REVERTED')
    reject(receipt.get('to','').lower()==TOKEN.lower() and receipt.get('from','').lower()==SOURCE.lower(),'BASE_RECEIPT_TX_MISMATCH')
    reject(tx.get('blockHash')==receipt.get('blockHash') and tx.get('blockNumber')==receipt.get('blockNumber'),'BASE_BLOCK_INCONSISTENT')
    current=int(rpc(rpc_url,'eth_blockNumber',[]),16)
    inclusion=int(receipt['blockNumber'],16)
    reject(current>=inclusion+MIN_BASE_CONFIRMATIONS-1,'BASE_TX_INSUFFICIENT_CONFIRMATIONS')
    # L2 block-count confirmations do not establish finality. Require Base's
    # finalized tag, which this deployment has verified on two RPC providers.
    finalized=rpc(rpc_url,'eth_getBlockByNumber',['finalized',False])
    reject(isinstance(finalized,dict) and isinstance(finalized.get('hash'),str),'BASE_FINALIZED_HEAD_UNAVAILABLE')
    finalized_height=int(finalized['number'],16)
    reject(finalized_height>=inclusion,'BASE_TX_NOT_L2_FINALIZED')
    block=rpc(rpc_url,'eth_getBlockByNumber',[receipt['blockNumber'],False])
    reject(isinstance(block,dict) and block.get('hash')==receipt['blockHash'],'BASE_REORG_OR_INVALID_BLOCK_HASH')
    inclusion_timestamp=int(block.get('timestamp','0x0'),16)
    reject(inclusion_timestamp>0,'BASE_INCLUSION_TIMESTAMP_MISSING')
    sender_topic='0x'+SOURCE[2:].lower().zfill(64)
    receiver_topic='0x'+destination[2:].lower().zfill(64)
    transfer=[]
    for event in receipt.get('logs',[]):
        topics=event.get('topics') or []
        if (event.get('address','').lower()==TOKEN.lower() and len(topics)==3 and topics[0].lower()==ERC20_TRANSFER_TOPIC
            and topics[1].lower()==sender_topic and topics[2].lower()==receiver_topic):
            try:val=int(event['data'],16)
            except (ValueError,KeyError,TypeError):continue
            if val==AMOUNT and event.get('transactionHash','').lower()==txid.lower():
                transfer.append(event)
    reject(len(transfer)==1,'BASE_EXACT_ERC20_TRANSFER_EVENT_NOT_FOUND_OR_DUPLICATE')
    code=rpc(rpc_url,'eth_getCode',[TOKEN,receipt['blockNumber']])
    reject(isinstance(code,str) and code.startswith('0x') and code!='0x','BASE_TOKEN_CODE_MISSING')
    reject(hashlib.sha256(bytes.fromhex(code[2:])).hexdigest()==TOKEN_CODE_SHA256,'BASE_TOKEN_CODE_MISMATCH')
    return {'tx_hash':txid.lower(),'chain_id':CHAIN_ID,'from':SOURCE.lower(),'token':TOKEN.lower(),
            'deposit_address':destination.lower(),'units':AMOUNT,'block_hash':receipt['blockHash'],
            'block_number':inclusion,'inclusion_timestamp_unix_s':inclusion_timestamp,
            'confirmations':current-inclusion+1,
            'finalized_block_height':finalized_height,'finalized_block_hash':finalized['hash'],
            'event_log_index':transfer[0].get('logIndex'),
            'token_code_sha256':TOKEN_CODE_SHA256,'rpc':rpc_url}


def verify_base_quorum(txid: str,destination: str) -> dict:
    observations=[require_base_tx(url,txid,destination) for url in BASE_RPCS]
    comparable=('tx_hash','chain_id','from','token','deposit_address','units','block_hash','block_number','event_log_index','token_code_sha256')
    reject(all(tuple(o[k] for k in comparable)==tuple(observations[0][k] for k in comparable) for o in observations[1:]),'BASE_RPC_RECEIPT_OR_LOG_DISAGREEMENT')
    reject(max(x['confirmations'] for x in observations)-min(x['confirmations'] for x in observations)<=MAX_BLOCK_DRIFT,'BASE_RPC_HEAD_DRIFT_TOO_HIGH')
    reject(max(x['finalized_block_height'] for x in observations)-min(x['finalized_block_height'] for x in observations)<=MAX_BLOCK_DRIFT,'BASE_RPC_FINALIZED_HEIGHT_DRIFT')
    if len({x['finalized_block_height'] for x in observations})==1:
        reject(len({x['finalized_block_hash'] for x in observations})==1,'BASE_RPC_FINALIZED_HASH_DISAGREEMENT')
    return {'classification':'independent_base_sepolia_finalized_transfer','observations':observations}


def verify_near_delivery(tx_hash: str, signer_id: str, *, min_units: int) -> dict:
    # Transaction status cannot be trusted without a successful finalized receipt and on-chain balance observations.
    reject(isinstance(tx_hash,str) and len(tx_hash)>=40,'NEAR_TX_HASH_REQUIRED')
    reject(isinstance(signer_id,str) and signer_id.endswith('.testnet'),'NEAR_TX_SIGNER_REQUIRED')
    status=rpc(NEAR_RPC,'status',[])
    reject(status.get('chain_id')=='testnet' and status.get('genesis_hash')==NEAR_GENESIS,'NEAR_WRONG_NETWORK')
    result=rpc(NEAR_RPC,'EXPERIMENTAL_tx_status',[tx_hash,signer_id])
    reject(isinstance(result,dict) and isinstance(result.get('transaction'),dict),'NEAR_TX_STATUS_MISSING')
    # FINAL is stronger than EXECUTED_OPTIMISTIC, EXECUTED, or INCLUDED_FINAL:
    # this receipt must be on the irreversible canonical NEAR chain.
    reject(result.get('final_execution_status')=='FINAL','NEAR_TRANSACTION_NOT_FINALIZED')
    reject(result['transaction'].get('hash')==tx_hash and result['transaction'].get('signer_id')==signer_id,'NEAR_TX_SENDER_OR_HASH_MISMATCH')
    outcomes=result.get('receipts_outcome') or []
    reject(outcomes,'NEAR_RECEIPTS_MISSING')
    reject(isinstance(result.get('status'),dict) and ('SuccessValue' in result['status'] or 'SuccessReceiptId' in result['status']),'NEAR_TX_NOT_SUCCESSFUL')
    for outcome in outcomes:
        o=outcome.get('outcome',{})
        reject('Failure' not in o.get('status',{}),'NEAR_RECEIPT_FAILED')
    indexed={o.get('id'):o for o in outcomes if isinstance(o,dict) and isinstance(o.get('id'),str)}
    delivered=[]
    for receipt in result.get('receipts') or []:
        if receipt.get('receiver_id')!=USDC_NEAR_CONTRACT:continue
        actions=(receipt.get('receipt') or {}).get('Action',{}).get('actions',[])
        for action in actions:
            call=action.get('FunctionCall') if isinstance(action,dict) else None
            if not isinstance(call,dict) or call.get('method_name')!='ft_transfer':continue
            try:args=json.loads(base64.b64decode(call['args'],validate=True))
            except (KeyError,ValueError,TypeError):continue
            if args.get('receiver_id')==RECEIVER and str(args.get('amount','')).isdecimal():
                matching=indexed.get(receipt.get('receipt_id'))
                reject(isinstance(matching,dict),'NEAR_TOKEN_TRANSFER_RECEIPT_OUTCOME_MISSING')
                outcome_status=(matching.get('outcome') or {}).get('status') or {}
                reject(isinstance(outcome_status,dict) and ('SuccessValue' in outcome_status or 'SuccessReceiptId' in outcome_status),'NEAR_TOKEN_TRANSFER_RECEIPT_NOT_SUCCESSFUL')
                proof_block_hash=matching.get('block_hash')
                reject(isinstance(proof_block_hash,str) and len(proof_block_hash)>30,'NEAR_TOKEN_TRANSFER_BLOCK_HASH_MISSING')
                block=rpc(NEAR_RPC,'block',{'block_id':proof_block_hash})
                reject(block.get('header',{}).get('hash')==proof_block_hash,'NEAR_TOKEN_TRANSFER_BLOCK_HASH_MISMATCH')
                height=block['header']['height']
                reject(isinstance(height,int) and height>0,'NEAR_TOKEN_TRANSFER_BLOCK_HEIGHT_MISSING')
                delivered.append({'amount':int(args['amount']),'block_hash':proof_block_hash,'block_height':height,'receipt_id':receipt.get('receipt_id')})
    reject(len(delivered)==1 and delivered[0]['amount']>=min_units,'NEAR_SIGNED_RECEIPT_DOES_NOT_PROVE_MINIMUM_DELIVERY')
    return {'classification':'near_finalized_transfer_receipt','transaction_hash':tx_hash,'signer':signer_id,
            'delivered_units':delivered[0]['amount'],'receiver':RECEIVER,'token':USDC_NEAR_CONTRACT,
            'receipt_count':len(outcomes),'transfer_receipt':delivered[0]}


def verify_delivery_balance(before_hash: str,after_hash: str, min_units: int) -> dict:
    reject(isinstance(before_hash,str) and len(before_hash)>30,'NEAR_PRE_SNAPSHOT_HASH_REQUIRED')
    reject(isinstance(after_hash,str) and len(after_hash)>30,'NEAR_POST_SNAPSHOT_HASH_REQUIRED')
    prior=near_ft_balance(RECEIVER,before_hash)
    after=near_ft_balance(RECEIVER,after_hash)
    reject(prior['block_hash']==before_hash and after['block_hash']==after_hash,'NEAR_SNAPSHOT_BLOCK_MISMATCH')
    reject(after['block_height']>prior['block_height'],'NEAR_BALANCE_BLOCK_ORDER_MISMATCH')
    # Supplied block IDs may exist on an optimistic/orphan branch. Validate
    # their canonical hashes BY HEIGHT, and the chain's finalized head.
    head=rpc(NEAR_RPC,'block',{'finality':'final'})
    final_height=(head.get('header') or {}).get('height')
    reject(isinstance(final_height,int) and final_height>=after['block_height'],'NEAR_SNAPSHOT_NOT_FINALIZED')
    for item in (prior,after):
        canonical=rpc(NEAR_RPC,'block',{'block_id':item['block_height']})
        reject((canonical.get('header') or {}).get('hash')==item['block_hash'],'NEAR_SNAPSHOT_NONCANONICAL_BLOCK')
    delta=after['units']-prior['units']
    reject(delta>=min_units,'NEAR_DESTINATION_BALANCE_INCREASE_BELOW_QUOTE_MINIMUM')
    return {'before':prior,'after':after,'net_increase_units':delta,
            'classification':'independent_near_state_balance_delta_not_sufficient_by_itself'}


def collect(quote_data: dict, *, guard_account: str, guard_sha: str, nonce: str,program_sha: str,
            base_tx_hash: str,near_tx_hash: str,near_tx_signer: str, before_hash: str,after_hash: str) -> dict:
    from base_sepolia_x3_intent import compile_testnet_intent
    source=compile_testnet_intent()
    reject(program_sha==source['program_sha256'],'X3_SOURCE_HASH_MISMATCH')
    reject(quote_data.get('classification')=='LIVE_QUOTE_UNFUNDED_UNAUTHORIZED_NOT_SETTLED','QUOTE_FILE_UNTRUSTED')
    # An actual historical executed quote will have expired *now*; validate maker
    # signature and signed terms against its issue timestamp, then require tx proof.
    maker_quote=quote_data['response']
    issued=datetime.fromisoformat(maker_quote['timestamp'].replace('Z','+00:00'))
    q=check_quote(maker_quote,issued)
    policy=get_near_policy(guard_account,RECEIVER,nonce,guard_sha)
    check_guard_policy(q,policy,owner=RECEIVER,nonce=nonce,now=issued,program_sha=program_sha)
    enforce_compiled_x3_policy(source,q,policy,nonce)
    base=verify_base_quorum(base_tx_hash,q['deposit_address'])
    # The EVM inclusion must be within the provider-signed settlement window
    # and the owner-signed Guard expiry. Never certify an expired deposit.
    source_inclusion=base['observations'][0]['inclusion_timestamp_unix_s']
    reject(source_inclusion>=int(q['issued_at'].timestamp())-120,'BASE_TRANSFER_PREDATES_SIGNED_QUOTE')
    reject(source_inclusion<=int(q['deadline'].timestamp()),'BASE_TRANSFER_AFTER_SIGNED_DEADLINE')
    reject(source_inclusion*1000<=int(policy['policy']['expiry_ms']),'BASE_TRANSFER_AFTER_OWNER_POLICY_EXPIRY')
    near=verify_near_delivery(near_tx_hash,near_tx_signer,min_units=q['minimum_out'])
    delta=verify_delivery_balance(before_hash,after_hash,q['minimum_out'])
    transfer_height=near['transfer_receipt']['block_height']
    reject(delta['before']['block_height']<transfer_height<=delta['after']['block_height'],
           'NEAR_TRANSFER_RECEIPT_OUTSIDE_SNAPSHOT_WINDOW')
    canonical=rpc(NEAR_RPC,'block',{'block_id':transfer_height})
    reject((canonical.get('header') or {}).get('hash')==near['transfer_receipt']['block_hash'],
           'NEAR_TRANSFER_RECEIPT_NONCANONICAL')
    return {'classification':'independent_testnet_two_chain_transfer_evidence_NOT_atomicity_or_native_x3vm_proof',
            'claim_limits':['Provider is custodial testnet relay','No cryptographic atomic HTLC proof','No mainnet readiness'],
            'quote_sha256':q['quote_sha256'],'program_sha256':program_sha,'guard_account':guard_account,
            'guard_wasm_sha256':guard_sha,'base_evidence':base,'near_receipt_evidence':near,
            'near_balance_evidence':delta,'refundable_source':SOURCE,
            'received_at_utc':datetime.now(timezone.utc).isoformat()}


def main() -> int:
    p=argparse.ArgumentParser(description=__doc__)
    for name in ['quote_file','guard_account','guard_wasm_sha256','nonce','program_sha256','base_tx_hash','near_tx_hash','near_tx_signer','near_balance_before_block','near_balance_after_block']:
        p.add_argument('--'+name.replace('_','-'),required=True,type=Path if name=='quote_file' else str)
    p.add_argument('--output',type=Path,required=True)
    args=p.parse_args()
    try:
        evidence=collect(json.loads(args.quote_file.read_text()),guard_account=args.guard_account,
            guard_sha=args.guard_wasm_sha256,nonce=args.nonce,program_sha=args.program_sha256,
            base_tx_hash=args.base_tx_hash,near_tx_hash=args.near_tx_hash,near_tx_signer=args.near_tx_signer,
            before_hash=args.near_balance_before_block,after_hash=args.near_balance_after_block)
        args.output.parent.mkdir(parents=True,exist_ok=True)
        data=(json.dumps(evidence,sort_keys=True,indent=2)+'\n').encode()
        # Exclusive creation atomically refuses racing evidence writers.
        with args.output.open('xb') as proof_file:
            proof_file.write(data)
            proof_file.flush()
        print(json.dumps({'classification':evidence['classification'],'proof_file':str(args.output),'proof_sha256':hashlib.sha256(data).hexdigest()},indent=2))
        return 0
    except (AdmissionError,ValueError,TypeError,KeyError,FileNotFoundError,UnicodeError) as e:
        print(json.dumps({'classification':'HARD_DENIED_NO_SETTLEMENT_CLAIM','reason':str(e)}),file=sys.stderr)
        return 2

if __name__=='__main__':raise SystemExit(main())
