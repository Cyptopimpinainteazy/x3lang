"""No-network tests of exact chain authorization and failure to sign on any missing proof."""
from __future__ import annotations

from copy import deepcopy
from datetime import datetime, timedelta, timezone
import importlib.util
import json
import base64
import hashlib
from pathlib import Path
import sys
import unittest
from unittest.mock import patch

ROOT=Path(__file__).resolve().parents[2]
sys.path.insert(0,str(ROOT/'x3-lang'))
p=Path(__file__).with_name('base_sepolia_guarded_admission.py')
spec=importlib.util.spec_from_file_location('guarded_admission',p)
m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m)

class TestReadOnlyAdmission(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        f=ROOT/'x3-lang/near/x3-tidepool-guard/src/signed_live_fixture.json'
        x=json.loads(f.read_text()); cls.response=json.loads(x['signed_payload']);cls.response['signature']=x['signature']
        cls.at=m.parse_time(cls.response['timestamp'])
        cls.deadline=m.parse_time(cls.response['quote']['deadline'])

    def test_real_signature_quote_admits_without_network_and_no_signature(self):
        info=m.check_quote(deepcopy(self.response),self.at)
        self.assertEqual(info['minimum_out'],987426)
        self.assertEqual(info['deposit_address'],'0x3e82c0843a6aE1D3837e1E17b6248c95799323a1')
        data=m.unsigned_calldata(info['deposit_address'],m.AMOUNT)
        self.assertTrue(data.startswith('0xa9059cbb'))
        self.assertEqual(len(data),138)
        self.assertNotIn('private',str(info).lower())

    def test_recipient_and_amount_tampering_fail(self):
        for field,where,value in [('recipient','quoteRequest','bad.testnet'),('amount','quoteRequest','2'),('amountIn','quote','2'),('depositAddress','quote','0x0000000000000000000000000000000000000001')]:
            c=deepcopy(self.response);c[where][field]=value
            with self.subTest(field=field),self.assertRaisesRegex(m.AdmissionError,'MAKER_SIGNATURE_INVALID'):
                m.check_quote(c,self.at)

    def test_quote_expiry_and_future_rejected(self):
        for delta in (timedelta(minutes=6),-timedelta(minutes=2)):
            with self.subTest(delta=delta),self.assertRaisesRegex(m.AdmissionError,'QUOTE_EXPIRED_OR_FUTURE'):
                m.check_quote(self.response,self.at+delta)

    def test_no_approval_never_admits_even_with_real_signature(self):
        info=m.check_quote(self.response,self.at)
        policy=self.policy(info)
        for mutation in ('missing','revoked','replay','owner','amount','expiry','refund','program','minimum'):
            p=deepcopy(policy)
            if mutation=='missing':p=None
            if mutation=='revoked':p['revoked']=True
            if mutation=='replay':p['quote_sha256']='0'*64
            if mutation=='owner':p['owner']='attacker.testnet'
            if mutation=='amount':p['policy']['amount_in']='2000000'
            if mutation=='expiry':p['policy']['expiry_ms']=0
            if mutation=='refund':p['policy']['refund_to']='0x0000000000000000000000000000000000000001'
            if mutation=='program':p['policy']['program_sha256']='c'*64
            if mutation=='minimum':p['policy']['minimum_out']='999999'
            with self.subTest(mutation=mutation),self.assertRaises(m.AdmissionError):
                m.check_guard_policy(info,p,owner=m.RECEIVER,nonce='demo00001',now=self.at,program_sha='a'*64)

    def test_valid_policy_matches_hash_from_authentic_payload(self):
        info=m.check_quote(self.response,self.at)
        m.check_guard_policy(info,self.policy(info),owner=m.RECEIVER,nonce='demo00001',now=self.at,program_sha='a'*64)

    def test_guard_code_unavailable_fails_closed(self):
        def fake(url,method,params):
            if method=='status':return {'chain_id':'testnet','genesis_hash':m.NEAR_GENESIS}
            raise m.AdmissionError('RPC_ACCOUNT_NOT_FOUND')
        with patch.object(m,'rpc',side_effect=fake),self.assertRaisesRegex(m.AdmissionError,'RPC_ACCOUNT_NOT_FOUND'):
            m.get_near_policy('x3-tidepool-guard.testnet',m.RECEIVER,'demo00001','f'*64)

    def test_wrong_genesis_fails_without_querying_contract(self):
        with patch.object(m,'rpc',return_value={'chain_id':'testnet','genesis_hash':'fake'}) as rpc:
            with self.assertRaisesRegex(m.AdmissionError,'GENESIS'):
                m.get_near_policy('x3-tidepool-guard.testnet',m.RECEIVER,'demo00001','f'*64)
            self.assertEqual(rpc.call_count,1)

    def test_guard_code_and_policy_read_same_exact_final_block(self):
        code=b'unit-test-wasm'
        sha=hashlib.sha256(code).hexdigest()
        block='ABC'+'1'*40
        row={'owner':m.RECEIVER,'nonce':'demo00001'}
        observations=[]
        def fake(url,method,params):
            observations.append((method,params))
            if method=='status': return {'chain_id':'testnet','genesis_hash':m.NEAR_GENESIS}
            if params.get('request_type')=='view_code':
                return {'code_base64':base64.b64encode(code).decode(),'block_hash':block,'block_height':123}
            self.assertEqual(params['request_type'],'call_function')
            self.assertEqual(params.get('block_id'),block)
            self.assertNotIn('finality',params)
            return {'block_hash':block,'result':list(json.dumps(row).encode())}
        with patch.object(m,'rpc',side_effect=fake):
            self.assertEqual(m.get_near_policy('guard.x3lang.testnet',m.RECEIVER,'demo00001',sha),row)
        self.assertEqual(len(observations),3)

    def test_guard_code_policy_block_mismatch_refused(self):
        code=b'unit-test-wasm'
        sha=hashlib.sha256(code).hexdigest()
        def fake(url,method,params):
            if method=='status':return {'chain_id':'testnet','genesis_hash':m.NEAR_GENESIS}
            if params.get('request_type')=='view_code':
                return {'code_base64':base64.b64encode(code).decode(),'block_hash':'A'*43}
            return {'block_hash':'B'*43,'result':list(b'{}')}
        with patch.object(m,'rpc',side_effect=fake),self.assertRaisesRegex(m.AdmissionError,'GUARD_CODE_POLICY_BLOCK_MISMATCH'):
            m.get_near_policy('guard.x3lang.testnet',m.RECEIVER,'demo00001',sha)

    def test_calldata_has_only_expected_transfer_selector(self):
        x=m.unsigned_calldata('0x00000000000000000000000000000000000000a1',1000000)
        self.assertEqual(x, '0xa9059cbb'+'00000000000000000000000000000000000000000000000000000000000000a1'+format(1000000,'064x'))

    @classmethod
    def policy(cls,info):
        return {'owner':m.RECEIVER,'nonce':'demo00001','revoked':False,'quote_sha256':info['quote_sha256'],
            'policy':{'program_sha256':'a'*64,'amount_in':'1000000','minimum_out':'980000',
                      'refund_to':m.SOURCE,'max_slippage_bps':100,'expiry_ms':int(cls.deadline.timestamp()*1000)+120000}}

if __name__=='__main__':unittest.main()
