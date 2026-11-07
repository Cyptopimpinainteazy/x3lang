"""Pinned REAL read-only public-chain baseline regression and synthetic RPC-denial tests.

Fixture is an actual finalized RPC observation. Mocked RPCs are confined to
synthetic negative unit tests; no fixture claims a swap was executed.
"""
import hashlib
import json
from pathlib import Path
import sys
import unittest
from unittest.mock import patch

ROOT=Path(__file__).resolve().parents[2]
sys.path.insert(0,str(ROOT/'x3-lang'))
sys.path.insert(0,str(Path(__file__).resolve().parent))
import capture_two_chain_baseline as cap
from base_sepolia_guarded_admission import AdmissionError

BASELINE=ROOT/'evidence/tidepool-testnet/pretrade-finalized-baseline-20261107.json'
PINNED_SHA256='69a835814805876acd2eafb1ca0390081e6bf4d4fca6bcc4c5347dbb609c4120'

class TestHistoricalBaseline(unittest.TestCase):
    def test_actual_baseline_bytes_unchanged(self):
        self.assertEqual(hashlib.sha256(BASELINE.read_bytes()).hexdigest(),PINNED_SHA256)

    def test_baseline_is_not_execution_proof(self):
        r=json.loads(BASELINE.read_text())
        self.assertEqual(r['classification'],'PRETRADE_TWO_CHAIN_FINALIZED_SAME_BLOCK_BALANCE_BASELINE_NOT_SWAP_EVIDENCE')
        self.assertIn('no trade executed',r['claims_excluded'])
        self.assertIn('no bridge/atomicity evidence',r['claims_excluded'])
        self.assertEqual(r['near']['finalized_balance']['units'],100_000_000)
        self.assertEqual(r['base']['quorum'][0]['token_balance_units'],100_000_000)
        self.assertEqual(r['base']['quorum'][0]['gas_balance_wei'],1_000_000_000_000_000)
        self.assertEqual(len({r['base']['quorum'][i]['finalized_block_hash'] for i in range(2)}),1)
        self.assertEqual(len({r['base']['quorum'][i]['finalized_block_height'] for i in range(2)}),1)
        self.assertEqual(r['base']['quorum'][0]['finalized_block_height'],55_543_788)
        self.assertEqual({q['rpc'] for q in r['base']['quorum']},set(cap.RPCS))

    def test_rpc_bad_finalized_hash_fails_closed_synthetic(self):
        def response(url,method,args):
            if method=='status':return {'chain_id':'testnet','genesis_hash':cap.NEAR_GENESIS}
            if method=='eth_chainId':return hex(cap.CHAIN_ID)
            if method=='eth_getBlockByNumber':
                return {'hash':('0x'+'a'*64) if url==cap.RPCS[0] else ('0x'+'b'*64),'number':'0x6000'}
            if method=='eth_getCode':return '0x60016002'
            if method=='eth_call':return hex(100000000)
            if method=='eth_getBalance':return hex(1000000000000000)
            raise AssertionError(method)
        with patch.object(cap,'rpc',side_effect=response), \
             patch.object(cap,'near_ft_balance',return_value={'units':100000000,'block_hash':'A'*43,'block_height':1}), \
             patch.object(cap,'TOKEN_CODE_SHA256',hashlib.sha256(bytes.fromhex('60016002')).hexdigest()), \
             self.assertRaisesRegex(AdmissionError,'BASE_FINALIZED_BLOCK_HASH_DISAGREEMENT'):
            cap.capture()

    def test_wrong_network_cannot_be_collected(self):
        with patch.object(cap,'rpc',return_value={'chain_id':'mainnet','genesis_hash':'wrong'}), \
             self.assertRaisesRegex(AdmissionError,'NEAR_GENESIS_MISMATCH'):
            cap.capture()

if __name__=='__main__':unittest.main()
