#!/usr/bin/env python3
"""Read-only finalized NEAR / two-provider Base Sepolia baseline, not swap proof."""
from __future__ import annotations
import argparse
import hashlib
import json
from datetime import datetime,timezone
from pathlib import Path
import sys
from base_sepolia_guarded_admission import (NEAR_RPC,NEAR_GENESIS,CHAIN_ID,SOURCE,TOKEN,TOKEN_CODE_SHA256,RECEIVER,
                                         AdmissionError,reject,rpc)
from two_chain_settlement_proof import near_ft_balance,USDC_NEAR_CONTRACT

# Pin two separately operated RPC providers; a failure of either hard-denies.
RPCS=('https://sepolia.base.org','https://base-sepolia-rpc.publicnode.com')


def capture() -> dict:
    status=rpc(NEAR_RPC,'status',[])
    reject(status.get('chain_id')=='testnet' and status.get('genesis_hash')==NEAR_GENESIS,'NEAR_GENESIS_MISMATCH')
    near=near_ft_balance(RECEIVER)
    # First negotiate a common finalized L2 height from independent providers.
    finalized=[]
    for url in RPCS:
        reject(int(rpc(url,'eth_chainId',[]),16)==CHAIN_ID,'BASE_SEPOLIA_CHAIN_MISMATCH')
        head=rpc(url,'eth_getBlockByNumber',['finalized',False])
        reject(isinstance(head,dict) and head.get('hash') and head.get('number'),'BASE_FINALIZED_HEAD_UNAVAILABLE')
        finalized.append({'rpc':url,'height':int(head['number'],16),'hash':head['hash']})
    reject(max(x['height'] for x in finalized)-min(x['height'] for x in finalized)<=6,'BASE_FINALIZED_RPC_DRIFT')
    common_height=min(x['height'] for x in finalized)
    block_id=hex(common_height)
    observations=[]
    for url in RPCS:
        block=rpc(url,'eth_getBlockByNumber',[block_id,False])
        reject(isinstance(block,dict) and block.get('hash'),'BASE_PINNED_BLOCK_UNAVAILABLE')
        code=rpc(url,'eth_getCode',[TOKEN,block_id])
        reject(isinstance(code,str) and code.startswith('0x') and code!='0x','BASE_TOKEN_CODE_MISSING')
        h=hashlib.sha256(bytes.fromhex(code[2:])).hexdigest()
        reject(h==TOKEN_CODE_SHA256,'BASE_TOKEN_WRONG_BYTECODE')
        bal=int(rpc(url,'eth_call',[{'to':TOKEN,'data':'0x70a08231'+SOURCE[2:].lower().zfill(64)},block_id]),16)
        eth=int(rpc(url,'eth_getBalance',[SOURCE,block_id]),16)
        observations.append({'rpc':url,'finalized_block_height':common_height,'finalized_block_hash':block['hash'],
                             'token_balance_units':bal,'gas_balance_wei':eth,'token_code_sha256':h})
    reject(len(observations)==2,'BASE_QUORUM_MISSING')
    reject(len({x['finalized_block_hash'] for x in observations})==1,'BASE_FINALIZED_BLOCK_HASH_DISAGREEMENT')
    reject(all((x['token_balance_units'],x['gas_balance_wei'],x['token_code_sha256']) ==
               (observations[0]['token_balance_units'],observations[0]['gas_balance_wei'],observations[0]['token_code_sha256']) for x in observations[1:]),'BASE_QUORUM_BALANCE_OR_CODE_MISMATCH')
    return {'classification':'PRETRADE_TWO_CHAIN_FINALIZED_SAME_BLOCK_BALANCE_BASELINE_NOT_SWAP_EVIDENCE',
            'captured_at_utc':datetime.now(timezone.utc).isoformat(),
            'near':{'genesis_hash':NEAR_GENESIS,'recipient':RECEIVER,'token':USDC_NEAR_CONTRACT,'finalized_balance':near},
            'base':{'chain_id':CHAIN_ID,'sender':SOURCE,'token':TOKEN,'quorum':observations},
            'claims_excluded':['no trade executed','no Guard deployed','no bridge/atomicity evidence','no mainnet readiness']}


def main() -> int:
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out',required=True,type=Path)
    args=parser.parse_args()
    try:
        reject(not args.out.exists(),'BASELINE_ALREADY_EXISTS_REFUSE_OVERWRITE')
        evidence=capture()
        args.out.parent.mkdir(parents=True,exist_ok=True)
        data=(json.dumps(evidence,sort_keys=True,indent=2)+'\n').encode()
        with args.out.open('xb') as f:f.write(data)
        print(json.dumps({'classification':evidence['classification'],'file':str(args.out),
                          'sha256':hashlib.sha256(data).hexdigest(),
                          'near_balance_units':evidence['near']['finalized_balance']['units'],
                          'near_finalized_block_hash':evidence['near']['finalized_balance']['block_hash'],
                          'base_USDC_units':evidence['base']['quorum'][0]['token_balance_units'],
                          'base_gas_wei':evidence['base']['quorum'][0]['gas_balance_wei']},indent=2))
        return 0
    except (AdmissionError,KeyError,TypeError,ValueError,OSError) as exc:
        print(json.dumps({'classification':'HARD_DENIED_NO_BASELINE','error':str(exc)}),file=sys.stderr)
        return 2

if __name__=='__main__':raise SystemExit(main())
