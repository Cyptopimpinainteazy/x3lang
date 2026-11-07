"""Adversarial tests over an ACTUAL archived first-swap certificate and quote.

Synthetic mutations in temporary local copies are not represented as chain evidence.
"""
from __future__ import annotations
import hashlib
import json
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

if __name__=='__main__':unittest.main()
