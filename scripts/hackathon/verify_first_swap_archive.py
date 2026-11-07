#!/usr/bin/env python3
"""Read-only, reproducible audit of X3's first finalized Tidepool testnet transfer.

Offline verifies authentic pinned maker-signed quote, bytes and internal evidence
relationships. --live rechecks historical Base block hash and canonical tx on
both RPCs, NEAR finalized receipt, historical owner Guard and balance deltas.
No credential access or transaction submission. Strict current-head quorum is
NOT replaced or bypassed: this audits an *already issued immutable certificate*.
"""
from __future__ import annotations
import argparse
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import sys
from base_sepolia_guarded_admission import (rpc, BASE_RPCS, CHAIN_ID, TOKEN, SOURCE,
    TOKEN_CODE_SHA256, AMOUNT, RECEIVER, check_quote, check_guard_policy,
    enforce_compiled_x3_policy, AdmissionError, reject)
from base_sepolia_x3_intent import compile_testnet_intent
from two_chain_settlement_proof import (verify_near_delivery,verify_delivery_balance,
    verify_historical_guard_approval,ERC20_TRANSFER_TOPIC)

ROOT=Path(__file__).resolve().parents[2]
CERT_PATH=ROOT/'evidence/tidepool-testnet/first-finalized-two-chain-certificate-20261107.json'
QUOTE_PATH=ROOT/'evidence/tidepool-testnet/first-signed-live-quote-20261107.json'
BASELINE_PATH=ROOT/'evidence/tidepool-testnet/pretrade-finalized-baseline-20261107.json'
PINNED_CERT_SHA='8e1baca995f4aa534821432c5e177f406eab6923f998473a36b4d5d9d5540a39'
PINNED_BASELINE_SHA='69a835814805876acd2eafb1ca0390081e6bf4d4fca6bcc4c5347dbb609c4120'
EXPECTED_SOURCE_TX='0x46c21dc305a9271c4e74c6d0734fb50f847a1e53d3cad87976133d66fd6728d9'
EXPECTED_DEST_TX='EBEBxJNgV9U5tBFBLj18ZzukgArqwXH5rBsBStFyCm1d'
EXPECTED_QUOTE_SHA='131f314c2532965f2678e79f3f7cc87cfe8238a549da1405e802f4c90db1034c'


def audited_files() -> tuple[dict,dict,dict,dict]:
    reject(hashlib.sha256(CERT_PATH.read_bytes()).hexdigest()==PINNED_CERT_SHA,'CERTIFICATE_HASH_MISMATCH')
    reject(hashlib.sha256(BASELINE_PATH.read_bytes()).hexdigest()==PINNED_BASELINE_SHA,'BASELINE_HASH_MISMATCH')
    cert=json.loads(CERT_PATH.read_text())
    historical=json.loads(BASELINE_PATH.read_text())
    signed=json.loads(QUOTE_PATH.read_text())
    source=compile_testnet_intent()
    reject(cert['classification']=='independent_testnet_two_chain_transfer_evidence_NOT_atomicity_or_native_x3vm_proof','FALSE_PROOF_CLASSIFICATION')
    reject(cert['program_sha256']==source['program_sha256'],'SOURCE_HASH_MISMATCH')
    reject(signed['classification']=='LIVE_QUOTE_UNFUNDED_UNAUTHORIZED_NOT_SETTLED','SOURCE_QUOTE_CLASSIFICATION_CHANGED')
    response=signed['response']
    issued=datetime.fromisoformat(response['timestamp'].replace('Z','+00:00'))
    quote=check_quote(response,issued)
    reject(quote['quote_sha256']==cert['quote_sha256']==EXPECTED_QUOTE_SHA,'SIGNED_QUOTE_HASH_MISMATCH')
    policy=cert['historical_guard_approval_evidence']['historical_policy']
    reject(policy['quote_sha256']==quote['quote_sha256'],'HISTORICAL_GUARD_QUOTE_MISMATCH')
    reject(cert['historical_guard_approval_evidence']['nonce']==source['nonce'],'HISTORICAL_NONCE_MISMATCH')
    check_guard_policy(quote,policy,owner=RECEIVER,nonce=source['nonce'],now=issued,program_sha=source['program_sha256'])
    enforce_compiled_x3_policy(source,quote,policy,source['nonce'])
    base=cert['base_evidence'];near=cert['near_receipt_evidence'];delta=cert['near_balance_evidence']
    reject(base['classification']=='independent_base_sepolia_finalized_transfer','BASE_PROOF_CLASSIFICATION_MISMATCH')
    reject(len(base['observations'])==2,'BASE_PROOF_NO_QUORUM')
    reject({x['rpc'] for x in base['observations']}==set(BASE_RPCS),'BASE_PROOF_WRONG_PROVIDERS')
    obs=base['observations']
    require_keys=('tx_hash','block_number','block_hash','finalized_block_height','finalized_block_hash','token_code_sha256','inclusion_timestamp_unix_s','event_log_index','units','deposit_address')
    reject(all(tuple(x[k] for k in require_keys)==tuple(obs[0][k] for k in require_keys) for x in obs[1:]),'ARCHIVED_BASE_QUORUM_DISAGREEMENT')
    reject(obs[0]['tx_hash']==EXPECTED_SOURCE_TX and obs[0]['units']==AMOUNT,'SOURCE_TRANSFER_MISMATCH')
    reject(obs[0]['deposit_address']==quote['deposit_address'].lower(),'SOURCE_DEPOSIT_MISMATCH')
    reject(obs[0]['token_code_sha256']==TOKEN_CODE_SHA256,'SOURCE_TOKEN_CODE_MISMATCH')
    reject(obs[0]['block_number']<=obs[0]['finalized_block_height'],'SOURCE_UNFINALIZED_IN_ARCHIVE')
    reject(obs[0]['confirmations']>=12,'SOURCE_CONFIRMATIONS_MISSING')
    reject(0<obs[0]['inclusion_timestamp_unix_s']<=int(quote['deadline'].timestamp()),'SOURCE_AFTER_DEADLINE')
    reject(obs[0]['inclusion_timestamp_unix_s']*1000<=int(policy['policy']['expiry_ms']),'SOURCE_AFTER_OWNER_EXPIRY')
    reject(cert['historical_guard_approval_evidence']['approval_timestamp_nanoseconds']//10**9<=obs[0]['inclusion_timestamp_unix_s'],'GUARD_APPROVED_AFTER_TRANSFER')
    reject(near['classification']=='near_finalized_transfer_receipt' and near['transaction_hash']==EXPECTED_DEST_TX,'NEAR_RECEIPT_MISMATCH')
    reject(near['receiver']==RECEIVER and near['delivered_units']==997400 and near['delivered_units']>=quote['minimum_out'],'NEAR_DELIVERY_MISMATCH')
    reject(delta['before']==historical['near']['finalized_balance'],'NEAR_BASELINE_CHANGED')
    reject(delta['net_increase_units']==near['delivered_units']==delta['after']['units']-delta['before']['units'],'NEAR_BALANCE_DELTA_MISMATCH')
    reject(delta['before']['block_height']<near['transfer_receipt']['block_height']<=delta['after']['block_height'],'NEAR_RECEIPT_OUTSIDE_DELIVERY_WINDOW')
    return cert,historical,signed,source


def audit_live(cert: dict) -> None:
    base=cert['base_evidence']['observations']
    agreed_block=base[0]['block_number']
    checkpoint_height=base[0]['finalized_block_height']
    checkpoint_hash=base[0]['finalized_block_hash']
    tx_hash=base[0]['tx_hash']
    transfer_topic='0x'+SOURCE[2:].lower().zfill(64)
    receiver_topic='0x'+base[0]['deposit_address'][2:].lower().zfill(64)
    for url in BASE_RPCS:
        reject(int(rpc(url,'eth_chainId',[]),16)==CHAIN_ID,'BASE_LIVE_WRONG_CHAIN')
        final=rpc(url,'eth_getBlockByNumber',['finalized',False])
        reject(isinstance(final,dict) and int(final['number'],16)>=checkpoint_height,'HISTORICAL_FINALITY_CHECKPOINT_NOT_FINALIZED')
        checkpoint=rpc(url,'eth_getBlockByNumber',[hex(checkpoint_height),False])
        reject(checkpoint.get('hash')==checkpoint_hash,'HISTORICAL_FINALITY_CHECKPOINT_REORGED')
        tx=rpc(url,'eth_getTransactionByHash',[tx_hash]);receipt=rpc(url,'eth_getTransactionReceipt',[tx_hash])
        reject(isinstance(tx,dict) and isinstance(receipt,dict),'ARCHIVED_TX_MISSING')
        reject(tx.get('from','').lower()==SOURCE.lower() and tx.get('to','').lower()==TOKEN.lower(),'ARCHIVED_TX_SOURCE_OR_CONTRACT_CHANGED')
        reject(int(tx.get('value','0x0'),16)==0,'ARCHIVED_UNEXPECTED_NATIVE_VALUE')
        calldata='0xa9059cbb'+base[0]['deposit_address'][2:].lower().zfill(64)+format(AMOUNT,'064x')
        reject(tx.get('input','').lower()==calldata,'ARCHIVED_CALLDATA_CHANGED')
        reject(int(receipt.get('status','0x0'),16)==1,'ARCHIVED_TRANSFER_NOT_SUCCESSFUL')
        reject(int(receipt['blockNumber'],16)==agreed_block and receipt['blockHash']==base[0]['block_hash'],'ARCHIVED_SOURCE_BLOCK_CHANGED')
        inc=rpc(url,'eth_getBlockByNumber',[hex(agreed_block),False])
        reject(inc.get('hash')==receipt['blockHash'] and int(inc['timestamp'],16)==base[0]['inclusion_timestamp_unix_s'],'ARCHIVED_INCLUSION_NOT_CANONICAL')
        logs=[x for x in receipt.get('logs',[]) if x.get('address','').lower()==TOKEN.lower()
              and [y.lower() for y in x.get('topics',[]) ]==[ERC20_TRANSFER_TOPIC,transfer_topic,receiver_topic]
              and int(x['data'],16)==AMOUNT]
        reject(len(logs)==1 and logs[0].get('logIndex')==base[0]['event_log_index'],'ARCHIVED_ERC20_TRANSFER_EVENT_CHANGED')
        code=rpc(url,'eth_getCode',[TOKEN,hex(agreed_block)])
        reject(hashlib.sha256(bytes.fromhex(code[2:])).hexdigest()==TOKEN_CODE_SHA256,'ARCHIVED_TOKEN_CODE_CHANGED')
    near=cert['near_receipt_evidence'];hist=cert['historical_guard_approval_evidence'];bal=cert['near_balance_evidence']
    actual_near=verify_near_delivery(near['transaction_hash'],near['signer'],min_units=near['delivered_units'])
    reject(actual_near==near,'NEAR_LIVE_RECEIPT_CHANGED')
    actual_delta=verify_delivery_balance(bal['before']['block_hash'],bal['after']['block_hash'],near['delivered_units'])
    reject(actual_delta==bal,'NEAR_LIVE_BALANCE_CHANGED')
    approval=verify_historical_guard_approval(cert['guard_account'],hist['owner'],hist['nonce'],
        cert['guard_wasm_sha256'],hist['approval_near_block_hash'],base[0]['inclusion_timestamp_unix_s'])
    reject(approval==hist,'NEAR_LIVE_GUARD_APPROVAL_CHANGED')


def main() -> int:
    ap=argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--live',action='store_true',help='read-only historical cross-provider re-verification')
    args=ap.parse_args()
    try:
        cert,baseline,quote,source=audited_files()
        if args.live:audit_live(cert)
        print(json.dumps({'result':'PASS','kind':'archived_first_guarded_tidepool_testnet_delivery',
          'mode':'live-historical-canonical' if args.live else 'offline-signed-evidence',
          'certificate_sha256':PINNED_CERT_SHA,'quote_sha256':cert['quote_sha256'],
          'base_tx_hash':EXPECTED_SOURCE_TX,'near_tx_hash':EXPECTED_DEST_TX,
          'sent_test_usdc_units':AMOUNT,'delivered_test_usdc_units':cert['near_receipt_evidence']['delivered_units'],
          'not_proven':['cryptographic cross-chain atomicity','native X3VM opcode execution','mainnet readiness']},indent=2))
        return 0
    except (AdmissionError,OSError,ValueError,KeyError,TypeError,UnicodeError) as exc:
        print(json.dumps({'result':'HARD_DENIED','reason':str(exc),'proof_changed':False}),file=sys.stderr)
        return 2

if __name__=='__main__':raise SystemExit(main())
