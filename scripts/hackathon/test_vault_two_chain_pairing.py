"""Synthetic metadata unit tests, NOT transaction/finality evidence."""
from __future__ import annotations
from dataclasses import replace
import copy
import unittest
from hashlib import sha256
from vault_origin_base_proof import Expected,ProofRejected
from near_delivery_receipt_proof import NearExpected
from vault_two_chain_pairing import x3_single_quote_base_nonce,correlate

class PairingTest(unittest.TestCase):
    def setUp(self):
        self.quote='1'*64;self.program='2'*64
        self.owner_nonce='x3_unique_new_owner_nonce'
        self.base_nonce=x3_single_quote_base_nonce(self.quote,self.program,self.owner_nonce)
        self.vault='0x'+'11'*20
        self.expected_base=Expected(self.vault,'0x'+'22'*20,'0x'+'33'*20,
                '0x'+'44'*20,'0x'+'55'*20,self.quote,self.program,'4'*64,'5'*64,
                self.base_nonce,1000,1800000100,1800000200,1000,1000)
        self.near=NearExpected('A'*43,'relayer.testnet','x3lang.testnet','usdc.tidepool-sandbox.testnet',
                900,'B'*43,'C'*43,'x3guardlaunch2026.testnet','6'*64,'D'*43,'J'*43,
                'x3lang.testnet',self.owner_nonce,self.quote,self.program,1000,1800000000,800,
                '0x'+'22'*20,100,1800000100)
        self.base={'classification':'READ_ONLY_BASE_VAULT_ORIGIN_FINALIZED_EVIDENCE_NOT_CROSSCHAIN_PROOF',
                   'independent_provider_count':2,'observations':[
                     {'tx_hash':'0x'+'ff'*32,'inclusion_unix_s':1800000000,'quote_hash':self.quote,
                      'program_hash':self.program,'deposit_address':'0x'+'55'*20} for i in range(2)]}
        self.near_report={'classification':'READ_ONLY_TWO_RPC_FINALIZED_NEAR_RECEIPT_BALANCE_AND_GUARD_NOT_ATOMICITY',
            'provider_count':2,'observations':[
                {'txhash':'A'*43,'quote_hash':self.quote,'program_hash':self.program,
                 'balance_delta':920,'guard_historical':{'approved_unix_s':1799999999},
                 'ft_receipt':{'unix_s':1800000200}} for i in range(2)]}

    def test_synthetic_pairing_remains_non_authoritative(self):
        r=correlate(self.base,self.near_report,self.expected_base,self.near)
        self.assertFalse(r['maker_signature_verified_by_this_module'])
        self.assertFalse(r['relayer_causality_cryptographically_proven'])
        self.assertFalse(r['trustless_atomic_settlement_proven'])
        self.assertFalse(r['broadcast'])

    def test_nonce_hash_matches_existing_x3_review_packet_domain(self):
        buf=(b'X3_SHIELD_SINGLE_QUOTE_BASE_VAULT_NONCE_V1\x00'+bytes.fromhex(self.quote)
             +bytes.fromhex(self.program)+self.owner_nonce.encode())
        self.assertEqual(self.base_nonce,'0x'+sha256(buf).hexdigest())

    def test_wrong_x3_nonce_amount_program_quote_or_time_fail(self):
        for mode in ('nonce','amount','quote','program','timestamp'):
            b=copy.deepcopy(self.base)
            e=self.expected_base
            n=self.near
            if mode=='nonce':n=replace(n,nonce='another')
            elif mode=='amount':n=replace(n,origin_amount=2000)
            elif mode=='quote':e=replace(e,quote_sha256='0'*64)
            elif mode=='program':e=replace(e,program_sha256='0'*64)
            elif mode=='timestamp':n=replace(n,source_inclusion_unix_s=1800000001)
            with self.subTest(mode=mode),self.assertRaises(ProofRejected):
                correlate(b,self.near_report,e,n)

    def test_unsupported_missing_near_and_late_guard_fail(self):
        for mode in ('classification','guard_late','balance','payout_before_source'):
            n=copy.deepcopy(self.near_report)
            if mode=='classification':n['classification']='TRUST_ME'
            elif mode=='guard_late':n['observations'][0]['guard_historical']['approved_unix_s']=1800000001
            elif mode=='balance':n['observations'][0]['balance_delta']=1
            elif mode=='payout_before_source':n['observations'][0]['ft_receipt']['unix_s']=1799999999
            with self.subTest(mode=mode),self.assertRaises(ProofRejected):
                correlate(self.base,n,self.expected_base,self.near)

if __name__=='__main__':unittest.main()
