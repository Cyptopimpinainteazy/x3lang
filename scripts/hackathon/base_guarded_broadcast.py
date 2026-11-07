#!/usr/bin/env python3
"""Single-process Guard-verified Base Sepolia testnet sender.

There is NO 'skip Guard' option. Owner must explicitly approve exact quote hash.
NO broadcasting is possible before real NEAR on-chain approval and gas checks.
Protected local Foundry keystore is used only after all checks pass.
"""
from __future__ import annotations

import argparse
import fcntl
import json
import os
from pathlib import Path
import subprocess
import sys
from datetime import datetime, timezone

sys.path.insert(0,str(Path(__file__).resolve().parent))
import base_sepolia_guarded_admission as admission
from base_sepolia_x3_intent import compile_testnet_intent

# No mainnet addresses, token IDs, RPCs, or credentials.
KEYSTORE_DIR=Path.home()/'.config/x3lang/base-sepolia-test-wallet'
KEYSTORE=KEYSTORE_DIR/'x3-base-sepolia-test'
PASSWORD_FILE=KEYSTORE_DIR/'password.txt'
RPC=admission.BASE_RPCS[0]
MAX_GAS_LIMIT=100_000
MAX_GAS_PRICE_WEI=1_000_000_000
MAX_FEE_WEI=100_000_000_000_000  # 0.0001 test ETH
CAST='/home/lojak/.foundry/bin/cast'


def assert_credentials() -> None:
    for path in (KEYSTORE,PASSWORD_FILE):
        admission.reject(path.is_file(),'OWNER_KEYSTORE_MISSING')
        admission.reject(path.stat().st_mode & 0o077 == 0,'OWNER_KEYSTORE_PERMISSIONS_UNSAFE')
    # Derive address from local encrypted keystore only after Guard verification.
    # This blocks accidentally executing from a different funded wallet.
    result=subprocess.run([CAST,'wallet','address','--keystore',str(KEYSTORE),
        '--password-file',str(PASSWORD_FILE)],capture_output=True,text=True,timeout=20,check=False)
    admission.reject(result.returncode==0 and result.stdout.strip().lower()==admission.SOURCE.lower(),
                     'KEYSTORE_ADDRESS_NOT_APPROVED_SOURCE')


def check_broadcast_gas(data: str) -> tuple[int,int,int]:
    admission.reject(int(admission.rpc(RPC,'eth_chainId',[]),16)==admission.CHAIN_ID,'WRONG_BROADCAST_RPC_CHAIN')
    estimate=int(admission.rpc(RPC,'eth_estimateGas',[{'from':admission.SOURCE,'to':admission.TOKEN,'data':data,'value':'0x0'}]),16)
    gas_price=int(admission.rpc(RPC,'eth_gasPrice',[]),16)
    nonce=int(admission.rpc(RPC,'eth_getTransactionCount',[admission.SOURCE,'pending']),16)
    admission.reject(0<estimate<=MAX_GAS_LIMIT,'GAS_ESTIMATE_EXCEEDS_CAP')
    admission.reject(0<gas_price<=MAX_GAS_PRICE_WEI,'GAS_PRICE_EXCEEDS_CAP')
    admission.reject(MAX_GAS_LIMIT*gas_price<=MAX_FEE_WEI,'GAS_BUDGET_EXCEEDS_CAP')
    return gas_price,nonce,estimate


def checked_admission(quote_file: Path, guard_account: str, guard_sha: str, owner_nonce: str, program_sha: str) -> dict:
    source=compile_testnet_intent()
    admission.reject(program_sha==source['program_sha256'],'X3_SOURCE_HASH_MISMATCH')
    raw=json.loads(quote_file.read_text())
    admission.reject(raw.get('classification')=='LIVE_QUOTE_UNFUNDED_UNAUTHORIZED_NOT_SETTLED','QUOTE_FILE_CLASSIFICATION_UNTRUSTED')
    now=datetime.now(timezone.utc)
    q=admission.check_quote(raw['response'],now)
    policy=admission.get_near_policy(guard_account,admission.RECEIVER,owner_nonce,guard_sha)
    admission.check_guard_policy(q,policy,owner=admission.RECEIVER,nonce=owner_nonce,now=now,program_sha=program_sha)
    state=admission.check_base_state()
    admission.reject(state['chain_id']==admission.CHAIN_ID,'WRONG_CHAIN')
    return q


def run_broadcast(*, quote_file: Path, guard_account: str, guard_sha: str, owner_nonce: str,
                  program_sha: str, approved_quote_sha: str, execute: bool) -> dict:
    admission.reject(len(approved_quote_sha)==64 and len(guard_sha)==64 and len(program_sha)==64,'EXPLICIT_HASH_APPROVALS_REQUIRED')
    # Local lock remains held until cast exits. Claim file is persisted BEFORE broadcast
    # to prevent accidental duplicate send after process crash or RPC timeout.
    KEYSTORE_DIR.mkdir(parents=True,exist_ok=True)
    lock_path=KEYSTORE_DIR/'x3guard-broadcast.lock'
    with lock_path.open('a+') as lock:
        fcntl.flock(lock,fcntl.LOCK_EX)
        q=checked_admission(quote_file,guard_account,guard_sha,owner_nonce,program_sha)
        admission.reject(q['quote_sha256']==approved_quote_sha,'USER_DID_NOT_APPROVE_THIS_QUOTE_HASH')
        data=admission.unsigned_calldata(q['deposit_address'],admission.AMOUNT)
        price,nonce,estimate=check_broadcast_gas(data)
        report={'classification':'GUARD_ADMITTED_TESTNET_ONLY','quote_sha256':q['quote_sha256'],
                'guard':guard_account,'base_chain_id':admission.CHAIN_ID,'source':admission.SOURCE,
                'token':admission.TOKEN,'deposit_address':q['deposit_address'],'amount':admission.AMOUNT,
                'tx_nonce':nonce,'estimated_gas':estimate,'gas_limit':MAX_GAS_LIMIT,'gas_price_wei':price,
                'max_fee_wei':MAX_GAS_LIMIT*price,'broadcast':False}
        if not execute:
            return report
        assert_credentials()
        flag=KEYSTORE_DIR/('submitted-'+q['quote_sha256']+'.json')
        admission.reject(not flag.exists(),'REPLAY_TRANSACTION_ALREADY_SUBMITTED_OR_ATTEMPTED')
        # Re-verify the *current* finalized Guard policy, signer quote and fees
        # directly before signing, not only before a potentially slow RPC series.
        fresh=checked_admission(quote_file,guard_account,guard_sha,owner_nonce,program_sha)
        admission.reject(fresh['quote_sha256']==approved_quote_sha and fresh['deposit_address']==q['deposit_address'],
                         'PRESIGN_QUOTE_OR_GUARD_CHANGED')
        price,nonce,estimate=check_broadcast_gas(data)
        report.update({'tx_nonce':nonce,'estimated_gas':estimate,'gas_price_wei':price,
                       'max_fee_wei':MAX_GAS_LIMIT*price})
        # Never log keystore password. No private key on CLI, stdout or in repo.
        # Mark as attempted first: a timeout must be investigated on-chain, not blindly retried.
        with flag.open('x') as out:
            os.fchmod(out.fileno(),0o600)
            json.dump({'status':'ATTEMPT_STARTED_REQUIRES_MANUAL_RECONCILIATION','nonce':nonce,'quote_sha256':q['quote_sha256']},out)
            out.flush();os.fsync(out.fileno())
        args=[CAST,'send',admission.TOKEN,'--data',data,'--rpc-url',RPC,
              '--keystore',str(KEYSTORE),'--password-file',str(PASSWORD_FILE),
              '--gas-limit',str(MAX_GAS_LIMIT),'--gas-price',str(price)+'wei',
              '--nonce',str(nonce),'--confirmations','2','--json']
        try:
            result=subprocess.run(args,capture_output=True,text=True,timeout=140,check=False)
        except subprocess.TimeoutExpired:
            # The node may have accepted a signed transaction before CLI timeout.
            # The attempt marker prevents a second send until on-chain reconciliation.
            return {**report,'classification':'BROADCAST_INDETERMINATE_CHECK_CHAIN_BEFORE_RETRY',
                    'broadcast':'unknown','reason':'CLI_TIMEOUT_AFTER_ATTEMPT'}
        if result.returncode:
            return {**report,'classification':'BROADCAST_INDETERMINATE_CHECK_CHAIN_BEFORE_RETRY',
                    'broadcast':'unknown','cast_returncode':result.returncode}
        try:
            tx=json.loads(result.stdout)
        except json.JSONDecodeError:
            return {**report,'classification':'BROADCAST_INDETERMINATE_CHECK_CHAIN_BEFORE_RETRY','broadcast':'unknown'}
        return {**report,'classification':'BROADCAST_RETURNED_RECEIPT_VERIFY_FINALITY_INDEPENDENTLY',
                'broadcast':True,'receipt':tx}


def main() -> int:
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--quote-file',required=True,type=Path)
    p.add_argument('--guard-account',required=True)
    p.add_argument('--guard-wasm-sha256',required=True)
    p.add_argument('--nonce',required=True)
    p.add_argument('--program-sha256',required=True)
    p.add_argument('--approve-exact-quote-sha256',required=True)
    p.add_argument('--execute',action='store_true',help='Actually broadcast testnet transfer only after all hard gates')
    args=p.parse_args()
    try:
        result=run_broadcast(quote_file=args.quote_file,guard_account=args.guard_account,
            guard_sha=args.guard_wasm_sha256,owner_nonce=args.nonce,program_sha=args.program_sha256,
            approved_quote_sha=args.approve_exact_quote_sha256,execute=args.execute)
        print(json.dumps(result,indent=2))
        return 0 if result['classification'] in ('GUARD_ADMITTED_TESTNET_ONLY','BROADCAST_RETURNED_RECEIPT_VERIFY_FINALITY_INDEPENDENTLY') else 2
    except (admission.AdmissionError,OSError,ValueError,KeyError,subprocess.TimeoutExpired) as exc:
        print(json.dumps({'classification':'HARD_DENIED_NO_BROADCAST','reason':str(exc)}),file=sys.stderr)
        return 2

if __name__=='__main__':raise SystemExit(main())
