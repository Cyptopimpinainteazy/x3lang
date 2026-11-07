#!/usr/bin/env python3
"""Fail-closed unsigned Base Sepolia transfer admission, bound to finalized NEAR Guard.

NO signing or broadcast path exists in this script. Only emit an unsigned ERC20
transfer plan after checking quote authenticity, owner authorization and RPC state.
The plan is NOT a permission slip for a separate signer without the same gates.
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
from urllib.request import Request, urlopen

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'x3-lang'))
from near_intents import _decode_ed25519, ed25519  # pinned reference implementation

NEAR_RPC = 'https://test.rpc.fastnear.com'
NEAR_GENESIS = 'FWJ9kR6KFWoyMoNjpLXXGHeuiy7tEY6GmoFeCA5yuc6b'
BASE_RPCS = ('https://base-sepolia.gateway.tenderly.co', 'https://base-sepolia.drpc.org')
CHAIN_ID = 84532
SOURCE = '0x5aF0c1BF95B3A158f6Fc48e3DED35903b2fa0721'
TOKEN = '0x38d81431B9f76d9699AF7517040D5221D9dEA252'
TOKEN_CODE_SHA256 = 'db77a21a647b5973bac5728bf9f6977b56933006bf2f0730a14d109d33072c83'
MAKER_KEY = 'ed25519:U1Mu4u1nHnHmgABpCkfDGXt2FZj8k29UGqhkMAu4iWY'
ORIGIN_ASSET = 'nep141:base-0x38d81431b9f76d9699af7517040d5221d9dea252.tp-omft.testnet'
DEST_ASSET = 'nep141:usdc.tidepool-sandbox.testnet'
RECEIVER = 'x3lang.testnet'
AMOUNT = 1_000_000  # one test USDC in 6-decimal base units
MAX_SLIPPAGE_BPS = 100
ERC20_SELECTOR = 'a9059cbb'  # transfer(address,uint256)

class AdmissionError(Exception):
    pass


def reject(condition: bool, message: str) -> None:
    if not condition:
        raise AdmissionError(message)


def post_json(url: str, payload: dict) -> dict:
    req=Request(url, data=json.dumps(payload,separators=(',',':')).encode(),headers={'Content-Type':'application/json','User-Agent':'X3Lang-guarded-read-only/1'})
    with urlopen(req, timeout=15) as resp:
        reject(resp.status == 200, f'HTTP_{resp.status}')
        data=json.load(resp)
        reject(isinstance(data,dict) and 'error' not in data, 'RPC_ERROR')
        return data


def rpc(url: str, method: str, params: list) -> Any:
    data=post_json(url,{'jsonrpc':'2.0','id':'x3-guard-readonly','method':method,'params':params})
    reject('result' in data, f'RPC_NO_RESULT_{method}')
    return data['result']


def parse_time(raw: str) -> datetime:
    reject(isinstance(raw,str) and raw.endswith('Z'), 'TIMESTAMP_MUST_BE_UTC_Z')
    try:
        result=datetime.fromisoformat(raw[:-1] + '+00:00')
    except ValueError as e:
        raise AdmissionError('INVALID_TIMESTAMP') from e
    return result


def signed_payload(response: dict) -> str:
    reject(isinstance(response,dict) and isinstance(response.get('signature'),str),'QUOTE_SIGNATURE_MISSING')
    return json.dumps({k:v for k,v in response.items() if k!='signature'},separators=(',',':'),ensure_ascii=False,allow_nan=False)


def check_quote(response: dict, now: datetime) -> dict:
    payload=signed_payload(response)
    try:
        signature=_decode_ed25519(response['signature'],64)
        key=_decode_ed25519(MAKER_KEY,32)
    except (TypeError,ValueError) as e:
        raise AdmissionError('INVALID_MAKER_SIGNATURE_ENCODING') from e
    reject(ed25519.verify(key,hashlib.sha256(payload.encode()).digest(),signature),'MAKER_SIGNATURE_INVALID')
    r=response.get('quoteRequest'); q=response.get('quote')
    reject(isinstance(r,dict) and isinstance(q,dict),'MALFORMED_SIGNED_QUOTE')
    fields={'dry':False,'swapType':'EXACT_INPUT','slippageTolerance':MAX_SLIPPAGE_BPS,
            'originAsset':ORIGIN_ASSET,'destinationAsset':DEST_ASSET,
            'depositType':'ORIGIN_CHAIN','refundType':'ORIGIN_CHAIN','recipientType':'DESTINATION_CHAIN',
            'recipient':RECEIVER,'amount':str(AMOUNT),'refundTo':SOURCE}
    for key,val in fields.items():
        reject(r.get(key)==val,f'SIGNED_QUOTE_FIELD_MISMATCH_{key}')
    try:
        actual=int(q['amountIn']); output=int(q['amountOut']); minimum=int(q['minAmountOut'])
    except (KeyError,TypeError,ValueError) as e:
        raise AdmissionError('SIGNED_QUOTE_INVALID_AMOUNTS') from e
    reject(actual==AMOUNT and output>0 and 0<minimum<=output,'SIGNED_QUOTE_AMOUNT_OUT_OF_BOUNDS')
    reject(minimum*10000 >= output*(10000-MAX_SLIPPAGE_BPS),'SIGNED_QUOTE_MINIMUM_TOO_LOW')
    dest=q.get('depositAddress')
    reject(isinstance(dest,str) and len(dest)==42 and dest.startswith('0x') and all(c in '0123456789abcdefABCDEF' for c in dest[2:]),'DEPOSIT_ADDRESS_INVALID')
    issued=parse_time(response['timestamp']);deadline=parse_time(q['deadline']); req_deadline=parse_time(r['deadline'])
    reject(-30 <= (now-issued).total_seconds() <= 300,'QUOTE_EXPIRED_OR_FUTURE')
    reject(60 <= (deadline-now).total_seconds() <= 3600,'QUOTE_DEADLINE_UNSAFE')
    reject(req_deadline>=deadline,'SIGNED_DEADLINE_MISMATCH')
    return {'quote_sha256':hashlib.sha256(payload.encode()).hexdigest(),'deposit_address':dest,'deadline':deadline,
            'amount_out':output,'minimum_out':minimum,'payload':payload}


def check_guard_policy(quote: dict, row: dict, *, owner: str, nonce: str, now: datetime, program_sha: str) -> None:
    reject(isinstance(row,dict),'NO_ONCHAIN_GUARD_POLICY')
    reject(row.get('owner')==owner and row.get('nonce')==nonce,'GUARD_OWNER_OR_NONCE_MISMATCH')
    reject(row.get('revoked') is False,'GUARD_REVOKED')
    reject(row.get('quote_sha256')==quote['quote_sha256'],'GUARD_QUOTE_NOT_APPROVED_ONCHAIN')
    p=row.get('policy');reject(isinstance(p,dict),'GUARD_POLICY_MISSING')
    reject(p.get('program_sha256')==program_sha,'PROGRAM_HASH_MISMATCH')
    reject(str(p.get('amount_in'))==str(AMOUNT),'GUARD_AMOUNT_MISMATCH')
    reject(p.get('refund_to')==SOURCE,'GUARD_REFUND_MISMATCH')
    reject(0 <= int(p.get('max_slippage_bps',-1)) <= MAX_SLIPPAGE_BPS,'GUARD_SLIPPAGE_MISMATCH')
    reject(int(p['max_slippage_bps']) >= MAX_SLIPPAGE_BPS,'GUARD_RESTRICTS_QUOTE_SLIPPAGE')
    reject(int(p.get('minimum_out','0'))>0 and int(p['minimum_out'])<=quote['minimum_out'],'GUARD_PAYOUT_MISMATCH')
    reject(int(p.get('expiry_ms',0))>int(now.timestamp()*1000),'GUARD_POLICY_EXPIRED')
    reject(int(p['expiry_ms'])>=int(quote['deadline'].timestamp()*1000),'GUARD_EXPIRES_BEFORE_QUOTE')


def get_near_policy(account: str,owner: str,nonce: str,expected_code_sha: str) -> dict:
    reject(account.endswith('.testnet') and account!=RECEIVER,'INVALID_TESTNET_GUARD_ACCOUNT')
    status=rpc(NEAR_RPC,'status',[])
    reject(status.get('chain_id')=='testnet' and status.get('genesis_hash')==NEAR_GENESIS,'NEAR_NETWORK_GENESIS_MISMATCH')
    code=rpc(NEAR_RPC,'query',{'request_type':'view_code','finality':'final','account_id':account})
    h=hashlib.sha256(base64.b64decode(code['code_base64'],validate=True)).hexdigest()
    reject(h==expected_code_sha,'UNAPPROVED_ONCHAIN_GUARD_CODE_HASH')
    args=base64.b64encode(json.dumps({'owner':owner,'nonce':nonce},separators=(',',':')).encode()).decode()
    view=rpc(NEAR_RPC,'query',{'request_type':'call_function','finality':'final','account_id':account,'method_name':'get_policy','args_base64':args})
    raw=bytes(view['result']).decode()
    return json.loads(raw)


def check_base_state() -> dict:
    states=[]
    for url in BASE_RPCS:
        chain=int(rpc(url,'eth_chainId',[]),16)
        reject(chain==CHAIN_ID,'BASE_WRONG_CHAIN')
        code=rpc(url,'eth_getCode',[TOKEN,'latest'])
        reject(code!='0x' and hashlib.sha256(bytes.fromhex(code[2:])).hexdigest()==TOKEN_CODE_SHA256,'BASE_WRONG_TOKEN_CODE')
        balance=int(rpc(url,'eth_call',[{'to':TOKEN,'data':'0x70a08231'+SOURCE[2:].lower().zfill(64)},'latest']),16)
        gas=int(rpc(url,'eth_getBalance',[SOURCE,'latest']),16)
        reject(balance>=AMOUNT,'BASE_TEST_USDC_INSUFFICIENT')
        reject(gas>0,'BASE_TEST_ETH_GAS_MISSING')
        states.append({'url':url,'balance':balance,'eth_wei':gas})
    reject(len({(s['balance'],s['eth_wei']) for s in states})==1,'BASE_RPC_DISAGREEMENT')
    return {'rpc_observations':states,'chain_id':CHAIN_ID}


def unsigned_calldata(destination: str, amount: int) -> str:
    return '0x'+ERC20_SELECTOR+destination[2:].lower().zfill(64)+format(amount,'064x')


def main() -> int:
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--quote-file',type=Path,required=True)
    parser.add_argument('--guard-account',required=True)
    parser.add_argument('--guard-wasm-sha256',required=True)
    parser.add_argument('--nonce',required=True)
    parser.add_argument('--program-sha256',required=True)
    a=parser.parse_args()
    try:
        reject(len(a.guard_wasm_sha256)==64 and len(a.program_sha256)==64,'EXPECTED_HASH_REQUIRED')
        raw=json.loads(a.quote_file.read_text())
        reject(raw.get('classification')=='LIVE_QUOTE_UNFUNDED_UNAUTHORIZED_NOT_SETTLED','QUOTE_FILE_CLASSIFICATION_UNTRUSTED')
        now=datetime.now(timezone.utc)
        quote=check_quote(raw['response'],now)
        policy=get_near_policy(a.guard_account,RECEIVER,a.nonce,a.guard_wasm_sha256)
        check_guard_policy(quote,policy,owner=RECEIVER,nonce=a.nonce,now=now,program_sha=a.program_sha256)
        state=check_base_state()
        print(json.dumps({'classification':'UNSIGNED_ADMITTED_TESTNET_TRANSFER_NOT_BROADCAST','source':SOURCE,'destination_near':RECEIVER,
          'origin_chain_id':CHAIN_ID,'token_contract':TOKEN,'deposit_address':quote['deposit_address'],
          'amount_base_units':AMOUNT,'guard_account':a.guard_account,'quote_sha256':quote['quote_sha256'],
          'erc20_calldata':unsigned_calldata(quote['deposit_address'],AMOUNT),'gas_state':state,'signed':False,'broadcast':False},indent=2))
        return 0
    except (AdmissionError,KeyError,TypeError,ValueError,OSError) as e:
        print(json.dumps({'classification':'HARD_DENIED','reason':str(e),'signed':False,'broadcast':False}),file=sys.stderr)
        return 2

if __name__=='__main__':raise SystemExit(main())
