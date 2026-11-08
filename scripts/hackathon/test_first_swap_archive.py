"""Adversarial tests over an ACTUAL archived first-swap certificate and quote.

Synthetic mutations in temporary local copies are not represented as chain evidence.
"""
from __future__ import annotations
import hashlib
import json
import io
import contextlib
from pathlib import Path
import shutil
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT=Path(__file__).resolve().parents[2]
sys.path.insert(0,str(ROOT/'x3-lang'))
sys.path.insert(0,str(Path(__file__).resolve().parent))
import verify_first_swap_archive as audit
from base_sepolia_guarded_admission import AdmissionError

class FirstSwapArchiveTests(unittest.TestCase):
    def test_original_certificate_verifies_offline(self):
        cert,baseline,signed,source=audit.audited_files()
        self.assertEqual(cert['near_receipt_evidence']['delivered_units'],997400)
        self.assertEqual(cert['base_evidence']['observations'][0]['units'],1000000)
        self.assertEqual(source['nonce'],'x3_base_testnet_trade_001')

    def test_cert_byte_change_fails_before_verification(self):
        with tempfile.TemporaryDirectory() as tmp:
            dest=Path(tmp)/'modified-cert.json'
            dest.write_bytes(audit.CERT_PATH.read_bytes()+b' ')
            with patch.object(audit,'CERT_PATH',dest),self.assertRaisesRegex(AdmissionError,'CERTIFICATE_HASH_MISMATCH'):
                audit.audited_files()

    def test_modified_historical_baseline_fails(self):
        with tempfile.TemporaryDirectory() as tmp:
            dest=Path(tmp)/'modified-baseline.json'
            dest.write_bytes(audit.BASELINE_PATH.read_bytes()+b' ')
            with patch.object(audit,'BASELINE_PATH',dest),self.assertRaisesRegex(AdmissionError,'BASELINE_HASH_MISMATCH'):
                audit.audited_files()

    def test_corrupted_signed_quote_signature_fails_crypto(self):
        with tempfile.TemporaryDirectory() as tmp:
            dest=Path(tmp)/'corrupted-quote.json'
            signed=json.loads(audit.QUOTE_PATH.read_text())
            signed['response']['signature']='ed25519:invalid'
            dest.write_text(json.dumps(signed))
            with patch.object(audit,'QUOTE_PATH',dest),self.assertRaises(AdmissionError):
                audit.audited_files()

    def test_live_recheck_rejects_reorg_at_archived_checkpoint(self):
        cert,_,_,_=audit.audited_files()
        def rpc_bad(url,method,params):
            if method=='eth_chainId':return hex(audit.CHAIN_ID)
            if method=='eth_getBlockByNumber':
                if params[0]=='finalized':return {'number':hex(55599000),'hash':'0x'+'f'*64}
                return {'number':params[0],'hash':'0x'+'0'*64}
            raise AssertionError((method,params))
        with patch.object(audit,'rpc',side_effect=rpc_bad),self.assertRaisesRegex(AdmissionError,'HISTORICAL_FINALITY_CHECKPOINT_REORGED'):
            audit.audit_live(cert)

    def test_original_classification_refuses_atomicity(self):
        data=json.loads(audit.CERT_PATH.read_text())
        self.assertIn('NOT_atomicity_or_native_x3vm_proof',data['classification'])
        self.assertIn('No cryptographic atomic HTLC proof',data['claim_limits'])
        self.assertIn('No mainnet readiness',data['claim_limits'])

    def test_live_replay_rejects_unbound_transaction_receipt_or_event(self):
        cert,_,_,_=audit.audited_files()
        observation=cert['base_evidence']['observations'][0]
        txid=observation['tx_hash']
        inclusion=hex(observation['block_number'])
        checkpoint=hex(observation['finalized_block_height'])
        calldata=('0xa9059cbb'+observation['deposit_address'][2:].lower().zfill(64)
                  +format(audit.AMOUNT,'064x'))
        token_code='0x60016002'
        valid_tx={'hash':txid,'from':audit.SOURCE,'to':audit.TOKEN,'value':'0x0',
                  'input':calldata,'blockHash':observation['block_hash'],'blockNumber':inclusion}
        valid_event={'address':audit.TOKEN,
                     'topics':[audit.ERC20_TRANSFER_TOPIC,
                               '0x'+audit.SOURCE[2:].lower().zfill(64),
                               '0x'+observation['deposit_address'][2:].lower().zfill(64)],
                     'data':hex(audit.AMOUNT),'logIndex':observation['event_log_index'],
                     'transactionHash':txid}
        valid_receipt={'transactionHash':txid,'status':'0x1','blockHash':observation['block_hash'],
                       'blockNumber':inclusion,'logs':[valid_event]}
        def check_case(mutation, bad_provider=None):
            tx=json.loads(json.dumps(valid_tx))
            receipt=json.loads(json.dumps(valid_receipt))
            if mutation=='tx_hash':tx['hash']='0x'+'f'*64
            elif mutation=='receipt_hash':receipt['transactionHash']='0x'+'f'*64
            elif mutation=='event_hash':receipt['logs'][0]['transactionHash']='0x'+'f'*64
            elif mutation=='tx_block_hash':tx['blockHash']='0x'+'e'*64
            elif mutation=='tx_block_number':tx['blockNumber']=hex(observation['block_number']+1)
            elif mutation=='tx_hash_null':tx['hash']=None
            elif mutation=='receipt_hash_null':receipt['transactionHash']=None
            elif mutation=='event_hash_null':receipt['logs'][0]['transactionHash']=None
            elif mutation=='tx_hash_numeric':tx['hash']=123
            elif mutation=='receipt_hash_numeric':receipt['transactionHash']=123
            elif mutation=='event_hash_numeric':receipt['logs'][0]['transactionHash']=123
            def fake_rpc(url,method,params):
                if method=='eth_chainId':return hex(audit.CHAIN_ID)
                if url==audit.NEAR_RPC and method=='block':
                    transfer=cert['near_receipt_evidence']['transfer_receipt']
                    self.assertEqual(params, {'block_id':transfer['block_height']})
                    return {'header':{'height':transfer['block_height'],
                                      'hash':'wrong' if mutation=='near_block' else transfer['block_hash']}}
                if method=='eth_getBlockByNumber':
                    if params[0]=='finalized':return {'number':hex(observation['finalized_block_height']+3)}
                    if params[0]==checkpoint:return {'hash':observation['finalized_block_hash']}
                    if params[0]==inclusion:
                        return {'hash':observation['block_hash'],'timestamp':hex(observation['inclusion_timestamp_unix_s'])}
                if method=='eth_getTransactionByHash':return tx if url==bad_provider else valid_tx
                if method=='eth_getTransactionReceipt':return receipt if url==bad_provider else valid_receipt
                if method=='eth_getCode':return token_code
                raise AssertionError((method,params))
            with patch.object(audit,'rpc',side_effect=fake_rpc), \
                 patch.object(audit,'TOKEN_CODE_SHA256',hashlib.sha256(bytes.fromhex(token_code[2:])).hexdigest()), \
                 patch.object(audit,'verify_near_delivery',return_value=cert['near_receipt_evidence']), \
                 patch.object(audit,'verify_delivery_balance',return_value=cert['near_balance_evidence']), \
                 patch.object(audit,'verify_historical_guard_approval',
                              return_value=cert['historical_guard_approval_evidence']):
                if mutation is None:
                    audit.audit_live(cert)  # Valid synthetic RPC control reaches PASS.
                elif mutation=='near_block':
                    with self.assertRaisesRegex(AdmissionError,'NEAR_TRANSFER_RECEIPT_NONCANONICAL'):
                        audit.audit_live(cert)
                else:
                    with self.assertRaises(AdmissionError):
                        audit.audit_live(cert)
                    if mutation.endswith(('_null','_numeric')):
                        error=io.StringIO()
                        with patch.object(sys,'argv',['replay','--live']),contextlib.redirect_stderr(error):
                            self.assertEqual(audit.main(),2)
                        self.assertEqual(json.loads(error.getvalue())['result'],'HARD_DENIED')
        check_case(None)
        check_case('near_block')
        for mutation in ('tx_hash','receipt_hash','event_hash','tx_block_hash','tx_block_number',
                         'tx_hash_null','receipt_hash_null','event_hash_null',
                         'tx_hash_numeric','receipt_hash_numeric','event_hash_numeric'):
            for provider in audit.BASE_RPCS:
                with self.subTest(mutation=mutation,provider=provider):
                    check_case(mutation,provider)


if __name__=='__main__':unittest.main()
