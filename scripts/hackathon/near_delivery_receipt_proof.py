#!/usr/bin/env python3
"""Strict read-only NEAR testnet transfer receipt + balance + historical Guard checks.

Synthetic test RPCs are for unit tests only, NOT chain evidence. Requires two
independent NEAR testnet RPC responses and canonical finalized blocks. Does not
cryptographically prove validator signatures or Tidepool's relayer causality.
"""
from __future__ import annotations

from dataclasses import dataclass
import base64
import json
from hashlib import sha256
from typing import Any,Callable
from urllib.parse import urlsplit

NEAR_GENESIS='FWJ9kR6KFWoyMoNjpLXXGHeuiy7tEY6GmoFeCA5yuc6b'
TOKEN='usdc.tidepool-sandbox.testnet'
DEST='x3lang.testnet'
Rpc=Callable[[str,Any],Any]

class NearProofRejected(ValueError):
    pass

def need(ok:bool,reason:str):
    if not ok: raise NearProofRejected(reason)

def is_hash(x:Any)->bool:
    # NEAR uses base58 hashes; validating here without misrepresenting them as hex.
    return isinstance(x,str) and 40<=len(x)<=52 and all(c in '123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz' for c in x)

def as_time_ns(block:dict)->int:
    h=block.get('header') or {}
    v=h.get('timestamp_nanosec',h.get('timestamp'))
    try:n=int(v)
    except (ValueError,TypeError) as exc:raise NearProofRejected('BLOCK_TIMESTAMP_MISSING') from exc
    need(n>=1_000_000_000_000,'INVALID_NEAR_TIMESTAMP')
    return n

def header_block(rpc:Rpc,ident:Any)->dict:
    b=rpc('block',{'block_id':ident} if ident!='final' else {'finality':'final'})
    need(isinstance(b,dict) and isinstance(b.get('header'),dict),'NEAR_BLOCK_NOT_FOUND')
    h=b['header']
    need(is_hash(h.get('hash')) and type(h.get('height')) is int and h['height']>0,'NEAR_HEADER_MALFORMED')
    if isinstance(ident,int):need(h['height']==ident,'NONCANONICAL_NEAR_HEIGHT')
    elif ident!='final':need(h['hash']==ident,'NONCANONICAL_NEAR_HASH')
    as_time_ns(b)
    return h

def view_balance(rpc:Rpc,token:str,account:str,block_hash:str)->tuple[int,int]:
    args=base64.b64encode(json.dumps({'account_id':account},separators=(',',':')).encode()).decode()
    row=rpc('query',{'request_type':'call_function','account_id':token,'method_name':'ft_balance_of',
                     'args_base64':args,'block_id':block_hash})
    need(isinstance(row,dict) and row.get('block_hash')==block_hash and type(row.get('block_height')) is int,
         'NEAR_BALANCE_WRONG_BLOCK')
    try:value=json.loads(bytes(row['result']).decode('utf-8'))
    except (ValueError,TypeError,KeyError) as exc:raise NearProofRejected('NEAR_BALANCE_MALFORMED') from exc
    need(isinstance(value,str) and value.isdecimal(),'NEAR_BALANCE_NONDECIMAL')
    return int(value),row['block_height']

@dataclass(frozen=True)
class NearExpected:
    txhash:str
    signer:str
    recipient:str
    token:str
    minimum_units:int
    before_block_hash:str
    after_block_hash:str
    guard_account:str
    guard_code_sha256:str
    guard_approval_block_hash:str
    guard_pre_source_block_hash:str
    owner:str
    nonce:str
    quote_sha256:str
    program_sha256:str
    origin_amount:int
    source_inclusion_unix_s:int
    compiled_minimum_units:int
    refund_to:str
    slippage_max_bps:int
    quote_deadline_unix_s:int
    max_settlement_lag_s:int=7200

    def validate(self):
        for h in (self.txhash,self.before_block_hash,self.after_block_hash,self.guard_approval_block_hash,self.guard_pre_source_block_hash):
            need(is_hash(h),'INVALID_NEAR_RECEIPT_OR_BLOCK_HASH')
        for a in (self.signer,self.recipient,self.token,self.guard_account,self.owner):
            need(isinstance(a,str) and a.endswith('.testnet') and len(a)<80,'NEAR_TESTNET_ACCOUNT_MISSING')
        need(self.token==TOKEN and self.recipient==DEST,'WRONG_NEAR_DESTINATION_ASSET_OR_RECIPIENT')
        need(type(self.minimum_units) is int and self.minimum_units>0,'NEAR_MINIMUM_OUT_INVALID')
        need(type(self.origin_amount) is int and self.origin_amount>0,'ORIGIN_AMOUNT_INVALID')
        need(type(self.compiled_minimum_units) is int and 0<self.compiled_minimum_units<=self.minimum_units,
             'INVALID_COMPILED_MINIMUM_RECEIVE')
        need(isinstance(self.refund_to,str) and len(self.refund_to)==42 and self.refund_to.startswith('0x')
             and all(c in '0123456789abcdefABCDEF' for c in self.refund_to[2:]),
             'INVALID_GUARD_REFUND_ADDRESS')
        need(type(self.slippage_max_bps) is int and 0<=self.slippage_max_bps<=100,'INVALID_SLIPPAGE_BOUND')
        need(type(self.quote_deadline_unix_s) is int and self.quote_deadline_unix_s>=self.source_inclusion_unix_s,
             'QUOTE_DEADLINE_BEFORE_ORIGIN_TX')
        need(type(self.source_inclusion_unix_s) is int and self.source_inclusion_unix_s>0,'SOURCE_TIMESTAMP_MISSING')
        need(type(self.max_settlement_lag_s) is int and 0<self.max_settlement_lag_s<=86400,'NEAR_MAX_LAG_INVALID')
        for x in (self.guard_code_sha256,self.quote_sha256,self.program_sha256):
            need(isinstance(x,str) and len(x)==64 and all(c in '0123456789abcdef' for c in x),'UNPINNED_GUARD_OR_QUOTE_HASH')
        need(isinstance(self.nonce,str) and 0<len(self.nonce)<=128,'MISSING_GUARD_NONCE')

def verify_guard_historical(rpc:Rpc,e:NearExpected,final_height:int)->dict:
    guard_header=header_block(rpc,e.guard_approval_block_hash)
    need(guard_header['height']<=final_height,'GUARD_APPROVAL_NOT_FINAL')
    canon=header_block(rpc,guard_header['height'])
    need(canon['hash']==e.guard_approval_block_hash,'GUARD_APPROVAL_NONCANONICAL')
    approved_at_ns=as_time_ns({'header':guard_header})
    need(approved_at_ns<=e.source_inclusion_unix_s*1_000_000_000,'GUARD_APPROVED_AFTER_BASE_PAYMENT')
    code=rpc('query',{'request_type':'view_code','block_id':e.guard_approval_block_hash,
                      'account_id':e.guard_account})
    need(isinstance(code,dict) and code.get('block_hash')==e.guard_approval_block_hash,'GUARD_WASM_CODE_WRONG_BLOCK')
    try:raw=base64.b64decode(code['code_base64'],validate=True)
    except (ValueError,KeyError) as exc:raise NearProofRejected('GUARD_WASM_CODE_MALFORMED') from exc
    need(len(raw)>0 and sha256(raw).hexdigest()==e.guard_code_sha256,'GUARD_WASM_CODE_MISMATCH')
    args=base64.b64encode(json.dumps({'owner':e.owner,'nonce':e.nonce},separators=(',',':')).encode()).decode()
    p=rpc('query',{'request_type':'call_function','block_id':e.guard_approval_block_hash,
                   'account_id':e.guard_account,'method_name':'get_policy','args_base64':args})
    need(isinstance(p,dict) and p.get('block_hash')==e.guard_approval_block_hash,'GUARD_POLICY_WRONG_BLOCK')
    try:policy=json.loads(bytes(p['result']).decode())
    except (ValueError,KeyError,TypeError) as exc:raise NearProofRejected('GUARD_POLICY_DECODE_FAILED') from exc
    need(isinstance(policy,dict) and policy.get('owner')==e.owner and policy.get('nonce')==e.nonce,
         'GUARD_OWNER_OR_NONCE_MISMATCH')
    need(policy.get('revoked') is False and policy.get('quote_sha256')==e.quote_sha256,
         'GUARD_QUOTE_NOT_APPROVED')
    terms=policy.get('policy') or {}
    need(isinstance(terms,dict) and terms.get('program_sha256')==e.program_sha256,
         'GUARD_PROGRAM_HASH_MISMATCH')
    need(str(terms.get('amount_in'))==str(e.origin_amount),'GUARD_ORIGIN_AMOUNT_MISMATCH')
    need(type(terms.get('expiry_ms')) in (str,int)
         and int(terms['expiry_ms'])>=e.quote_deadline_unix_s*1000,
         'GUARD_OWNER_MANDATE_EXPIRED_BEFORE_QUOTE_DEADLINE')
    minimum=terms.get('minimum_out')
    need(type(minimum) in (str,int) and str(minimum).isdecimal()
         and e.compiled_minimum_units<=int(minimum)<=e.minimum_units,
         'GUARD_MINIMUM_BELOW_X3_PROGRAM_FLOOR')
    need(isinstance(terms.get('refund_to'),str)
         and terms['refund_to'].lower()==e.refund_to.lower(),
         'GUARD_REFUND_NOT_APPROVED_OWNER')
    slippage=terms.get('max_slippage_bps')
    need(type(slippage) in (str,int) and str(slippage).isdecimal()
         and int(slippage)==e.slippage_max_bps,
         'GUARD_SLIPPAGE_DOES_NOT_MATCH_SIGNED_QUOTE')
    # Confirm the historical approval remained active at the LAST canonical
    # NEAR block before the Base inclusion second. An older approved state
    # alone cannot rule out a subsequent on-chain revocation or code upgrade.
    pre=header_block(rpc,e.guard_pre_source_block_hash)
    need(guard_header['height']<=pre['height']<=final_height,
         'GUARD_PRE_SOURCE_BLOCK_OUT_OF_ORDER')
    need(header_block(rpc,pre['height'])['hash']==e.guard_pre_source_block_hash,
         'GUARD_PRE_SOURCE_BLOCK_NONCANONICAL')
    pre_ns=as_time_ns({'header':pre})
    source_ns=e.source_inclusion_unix_s*1_000_000_000
    need(approved_at_ns<=pre_ns<=source_ns,'GUARD_PRE_SOURCE_STATE_AFTER_PAYMENT')
    successor=header_block(rpc,pre['height']+1)
    need(as_time_ns({'header':successor})>source_ns,
         'GUARD_PRE_SOURCE_STATE_NOT_LAST_CANONICAL_BLOCK')
    # The next block must itself be canonical: an orphan successor must not
    # masquerade as evidence that no further revocation occurred.
    need(header_block(rpc,successor['height'])['hash']==successor['hash'],
         'GUARD_PRE_SOURCE_NEXT_BLOCK_NONCANONICAL')
    prec=rpc('query',{'request_type':'view_code','block_id':e.guard_pre_source_block_hash,
                      'account_id':e.guard_account})
    need(isinstance(prec,dict) and prec.get('block_hash')==e.guard_pre_source_block_hash,
         'GUARD_PRE_SOURCE_CODE_WRONG_BLOCK')
    try:pre_code=base64.b64decode(prec['code_base64'],validate=True)
    except (ValueError,KeyError,TypeError) as exc:raise NearProofRejected('GUARD_PRE_SOURCE_CODE_MALFORMED') from exc
    need(len(pre_code)>0 and sha256(pre_code).hexdigest()==e.guard_code_sha256,
         'GUARD_PRE_SOURCE_CODE_CHANGED')
    pv=rpc('query',{'request_type':'call_function','block_id':e.guard_pre_source_block_hash,
                    'account_id':e.guard_account,'method_name':'get_policy','args_base64':args})
    need(isinstance(pv,dict) and pv.get('block_hash')==e.guard_pre_source_block_hash,
         'GUARD_PRE_SOURCE_POLICY_WRONG_BLOCK')
    try:pre_policy=json.loads(bytes(pv['result']).decode())
    except (ValueError,KeyError,TypeError) as exc:raise NearProofRejected('GUARD_PRE_SOURCE_POLICY_DECODE_FAILED') from exc
    need(pre_policy==policy and pre_policy.get('revoked') is False,
         'GUARD_PRE_SOURCE_POLICY_CHANGED_OR_REVOKED')
    return {'block_hash':e.guard_approval_block_hash,'block_height':guard_header['height'],
            'approved_unix_s':approved_at_ns//1_000_000_000,'code_sha256':e.guard_code_sha256,
            'policy_sha256':sha256(json.dumps(policy,sort_keys=True,separators=(',',':')).encode()).hexdigest(),
            'pre_source_block_hash':e.guard_pre_source_block_hash,
            'pre_source_block_height':pre['height'],
            'next_block_height':successor['height'],
            'active_through_base_inclusion_second_verified':True}

def verify_one(rpc:Rpc,e:NearExpected)->dict:
    e.validate()
    status=rpc('status',[])
    need(isinstance(status,dict) and status.get('chain_id')=='testnet' and status.get('genesis_hash')==NEAR_GENESIS,
         'NEAR_GENESIS_MISMATCH')
    head=header_block(rpc,'final')
    record=rpc('EXPERIMENTAL_tx_status',[e.txhash,e.signer])
    need(isinstance(record,dict) and record.get('final_execution_status')=='FINAL','NEAR_TRANSACTION_NOT_FINAL')
    tx=record.get('transaction') or {}
    need(tx.get('hash')==e.txhash and tx.get('signer_id')==e.signer,'NEAR_TX_SIGNER_OR_HASH_MISMATCH')
    state=record.get('status') or {}
    need(isinstance(state,dict) and ('SuccessValue' in state or 'SuccessReceiptId' in state),
         'NEAR_TX_NOT_SUCCESSFUL')
    outcomes=record.get('receipts_outcome') or []
    need(isinstance(outcomes,list) and len(outcomes)>0,'NEAR_RECEIPT_OUTCOMES_MISSING')
    out_by_id={}
    for o in outcomes:
        status=(o.get('outcome') or {}).get('status') or {}
        need(isinstance(status,dict) and ('SuccessValue' in status or 'SuccessReceiptId' in status),
             'NEAR_RECEIPT_OUTCOME_FAILED')
        out_by_id[o.get('id')]=o
    deliveries=[]
    for receipt in record.get('receipts') or []:
        if receipt.get('receiver_id')!=e.token:continue
        for a in ((receipt.get('receipt') or {}).get('Action') or {}).get('actions',[]):
            call=a.get('FunctionCall') if isinstance(a,dict) else None
            if not isinstance(call,dict) or call.get('method_name')!='ft_transfer':continue
            try:args=json.loads(base64.b64decode(call['args'],validate=True))
            except (ValueError,KeyError,TypeError) as exc:raise NearProofRejected('NEAR_FT_CALL_MALFORMED') from exc
            if args.get('receiver_id')!=e.recipient:continue
            amount=args.get('amount')
            need(isinstance(amount,str) and amount.isdecimal(),'NEAR_FT_TRANSFER_AMOUNT_BAD')
            outcome=out_by_id.get(receipt.get('receipt_id'))
            need(isinstance(outcome,dict),'NEAR_FT_RECEIPT_LACKS_FINAL_OUTCOME')
            block_hash=outcome.get('block_hash')
            need(is_hash(block_hash),'NEAR_FT_TRANSFER_BLOCK_MISSING')
            h=header_block(rpc,block_hash)
            need(h['height']<=head['height'],'NEAR_FT_TRANSFER_NOT_FINAL')
            need(header_block(rpc,h['height'])['hash']==block_hash,'NEAR_FT_TRANSFER_NONCANONICAL')
            deliveries.append({'amount':int(amount),'receipt_id':receipt['receipt_id'],
                               'block_hash':block_hash,'height':h['height'],'unix_s':as_time_ns({'header':h})//1_000_000_000})
    need(len(deliveries)==1 and deliveries[0]['amount']>=e.minimum_units,
         'NEAR_EXPECTED_FT_DELIVERY_MISSING_OR_DUPLICATE')
    delivery=deliveries[0]
    need(e.source_inclusion_unix_s<=delivery['unix_s']<=e.source_inclusion_unix_s+e.max_settlement_lag_s,
         'NEAR_PAYOUT_TIME_ORDER_OR_TIMEOUT_INVALID')
    before_h=header_block(rpc,e.before_block_hash)
    after_h=header_block(rpc,e.after_block_hash)
    need(before_h['height']<delivery['height']<=after_h['height']<=head['height'],
         'NEAR_BALANCE_SNAPSHOTS_DO_NOT_ENCLOSE_TRANSFER')
    for header in (before_h,after_h):
        need(header_block(rpc,header['height'])['hash']==header['hash'],'NONCANONICAL_NEAR_BALANCE_BLOCK')
    before,bheight=view_balance(rpc,e.token,e.recipient,e.before_block_hash)
    after,aheight=view_balance(rpc,e.token,e.recipient,e.after_block_hash)
    need(bheight==before_h['height'] and aheight==after_h['height'],
         'NEAR_BALANCE_QUERY_HEIGHT_MISMATCH')
    need(after>=before and after-before>=e.minimum_units,'NEAR_DESTINATION_BALANCE_DELTA_INSUFFICIENT')
    guard=verify_guard_historical(rpc,e,head['height'])
    return {'chain_id':'testnet','genesis_hash':NEAR_GENESIS,'txhash':e.txhash,
            'ft_receipt':delivery,'balance_before':before,'balance_after':after,'balance_delta':after-before,
            'before_block_hash':e.before_block_hash,'after_block_hash':e.after_block_hash,
            'guard_historical':guard,'head_height':head['height'],'head_hash':head['hash'],
            'quote_hash':e.quote_sha256,'program_hash':e.program_sha256,'recipient':e.recipient,
            'min_units':e.minimum_units,'token':e.token}

def verify_two_providers(observations:tuple[tuple[str,Rpc],tuple[str,Rpc]], e:NearExpected)->dict:
    need(len(observations)==2,'TWO_NEAR_RPC_REQUIRED')
    hosts=[urlsplit(u) for u,_ in observations]
    need(all(p.scheme=='https' and p.hostname and not p.username and not p.password
             and not p.query and not p.fragment for p in hosts),
         'NEAR_RPC_URL_INVALID')
    need(hosts[0].hostname!=hosts[1].hostname,'NEAR_RPC_PROVIDERS_NOT_INDEPENDENT')
    a=verify_one(observations[0][1],e)
    b=verify_one(observations[1][1],e)
    shared=[k for k in a if k not in ('head_height','head_hash')]
    need(all(a[k]==b[k] for k in shared),'NEAR_FINALIZED_RPC_EVIDENCE_DISAGREEMENT')
    need(abs(a['head_height']-b['head_height'])<=60,'NEAR_FINALIZED_HEAD_DRIFT')
    if a['head_height']==b['head_height']:
        need(a['head_hash']==b['head_hash'],'NEAR_FINALIZED_HEAD_HASH_MISMATCH')
    return {'classification':'READ_ONLY_TWO_RPC_FINALIZED_NEAR_RECEIPT_BALANCE_AND_GUARD_NOT_ATOMICITY',
            'provider_count':2,'provider_hosts':[p.hostname for p in hosts],
            'observations':[a,b],
            'cryptographic_validator_finality_proven':False,
            'relayer_causality_proven':False,'signed':False,'broadcast':False}

# Intentionally no network transport or CLI: this module should be integrated
# with the existing project's tightly reviewed NEAR RPC readers after review.

def _https_near_rpc(url:str)->Rpc:
    from urllib.request import Request,urlopen
    parsed=urlsplit(url)
    need(parsed.scheme=='https' and parsed.hostname and not parsed.username and not parsed.password
         and not parsed.query and not parsed.fragment,'NEAR_RPC_MUST_USE_CREDENTIAL_FREE_HTTPS')
    methods={'status','block','query','EXPERIMENTAL_tx_status'}
    def read(method:str,params:Any)->Any:
        need(method in methods,'NEAR_RPC_WRITE_METHOD_FORBIDDEN')
        payload=json.dumps({'jsonrpc':'2.0','id':'x3-vault-near-readonly','method':method,'params':params},
            separators=(',',':'),allow_nan=False).encode()
        req=Request(url,data=payload,headers={'Content-Type':'application/json','User-Agent':'X3Shield-Readonly-NEAR-Proof/1'})
        try:
            with urlopen(req,timeout=15) as resp:
                need(resp.status==200,'NEAR_RPC_NON_200')
                row=json.load(resp)
        except (OSError,ValueError) as exc:
            raise NearProofRejected('NEAR_RPC_UNAVAILABLE_'+method) from exc
        need(isinstance(row,dict) and row.get('jsonrpc')=='2.0' and row.get('id')=='x3-vault-near-readonly'
             and 'error' not in row and 'result' in row,'NEAR_RPC_MALFORMED_'+method)
        return row['result']
    return read
