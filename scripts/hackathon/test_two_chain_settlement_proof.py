"""Synthetic adversarial fixtures, NOT live chain settlement evidence."""
from __future__ import annotations
from copy import deepcopy
from datetime import datetime,timezone,timedelta
import importlib.util
import json
from pathlib import Path
import sys
import unittest
from unittest.mock import patch

ROOT=Path(__file__).resolve().parents[2]
sys.path.insert(0,str(ROOT/'x3-lang'))
sys.path.insert(0,str(Path(__file__).resolve().parent))
spec=importlib.util.spec_from_file_location('chain_proof',Path(__file__).with_name('two_chain_settlement_proof.py'))
p=importlib.util.module_from_spec(spec);spec.loader.exec_module(p)

SOURCE=p.SOURCE
TO='0x3e82c0843a6aE1D3837e1E17b6248c95799323a1'
TXID='0x'+'a'*64
BLOCK='0x'+'b'*64
TRANSFER='0xa9059cbb'+TO[2:].lower().zfill(64)+format(p.AMOUNT,'064x')


def fixtures():
    tx={'hash':TXID,'from':SOURCE,'to':p.TOKEN,'value':'0x0','input':TRANSFER,'blockHash':BLOCK,'blockNumber':hex(1000)}
    event={'address':p.TOKEN,'topics':[p.ERC20_TRANSFER_TOPIC,'0x'+SOURCE[2:].lower().zfill(64),'0x'+TO[2:].lower().zfill(64)],
           'data':hex(p.AMOUNT),'transactionHash':TXID,'logIndex':'0x1'}
    receipt={'transactionHash':TXID,'from':SOURCE,'to':p.TOKEN,'status':'0x1','blockHash':BLOCK,'blockNumber':hex(1000),'logs':[event]}
    code='0x60016002'
    return tx,receipt,event,code


class ProofSecurityTests(unittest.TestCase):
    def check_base(self,edit=None):
        tx,receipt,event,code=fixtures()
        if edit:edit(tx,receipt,event)
        def fake(url,method,params):
            if method=='eth_chainId':return hex(p.CHAIN_ID)
            if method=='eth_getTransactionByHash':return tx
            if method=='eth_getTransactionReceipt':return receipt
            if method=='eth_blockNumber':return hex(1030)
            if method=='eth_getBlockByNumber':
                return {'number':hex(1020),'hash':'0x'+'f'*64} if params[0]=='finalized' else {'hash':BLOCK}
            if method=='eth_getCode':return code
            raise AssertionError('Unexpected '+method)
        with patch.object(p,'rpc',side_effect=fake),patch.object(p,'TOKEN_CODE_SHA256',__import__('hashlib').sha256(bytes.fromhex(code[2:])).hexdigest()):
            return p.require_base_tx('https://test.invalid',TXID,TO)

    def test_exact_transfer_receipt_positive_synthetic(self):
        x=self.check_base()
        self.assertEqual(x['units'],1_000_000)
        self.assertEqual(x['block_hash'],BLOCK)
        self.assertGreaterEqual(x['confirmations'],12)

    def test_reverted_receipt_rejected(self):
        with self.assertRaisesRegex(p.AdmissionError,'BASE_TX_REVERTED'):
            self.check_base(lambda tx,receipt,event:receipt.update(status='0x0'))

    def test_unfinalized_base_receipt_rejected_despite_confirmations(self):
        tx,receipt,event,code=fixtures()
        def fake(url,method,params):
            if method=='eth_chainId':return hex(p.CHAIN_ID)
            if method=='eth_getTransactionByHash':return tx
            if method=='eth_getTransactionReceipt':return receipt
            if method=='eth_blockNumber':return hex(1030)
            if method=='eth_getBlockByNumber':return {'number':hex(999),'hash':'0x'+'f'*64}
            raise AssertionError(method)
        with patch.object(p,'rpc',side_effect=fake),self.assertRaisesRegex(p.AdmissionError,'BASE_TX_NOT_L2_FINALIZED'):
            p.require_base_tx('https://test.invalid',TXID,TO)

    def test_wrong_transfer_destination_rejected(self):
        with self.assertRaisesRegex(p.AdmissionError,'BASE_EXACT_ERC20_TRANSFER_EVENT_NOT_FOUND'):
            self.check_base(lambda tx,receipt,event:event['topics'].__setitem__(2,'0x'+'0'*64))

    def test_unexpected_source_rejected(self):
        with self.assertRaisesRegex(p.AdmissionError,'BASE_SENDER_OR_CONTRACT_MISMATCH'):
            self.check_base(lambda tx,receipt,event:tx.update(**{'from':'0x'+'f'*40}))

    def test_unexpected_value_rejected(self):
        with self.assertRaisesRegex(p.AdmissionError,'BASE_UNEXPECTED_NATIVE_VALUE'):
            self.check_base(lambda tx,receipt,event:tx.update(value='0x1'))

    def test_wrong_calldata_amount_rejected(self):
        with self.assertRaisesRegex(p.AdmissionError,'BASE_TRANSFER_CALLDATA_MISMATCH'):
            self.check_base(lambda tx,receipt,event:tx.update(input=TRANSFER[:-2]+'ff'))

    def test_duplicate_logs_rejected(self):
        with self.assertRaisesRegex(p.AdmissionError,'BASE_EXACT_ERC20_TRANSFER_EVENT_NOT_FOUND_OR_DUPLICATE'):
            self.check_base(lambda tx,receipt,event:receipt['logs'].append(dict(event)))

    def test_insufficient_confirmations_rejected(self):
        tx,receipt,event,code=fixtures()
        def fake(url,method,params):
            d={'eth_chainId':hex(p.CHAIN_ID),'eth_getTransactionByHash':tx,
               'eth_getTransactionReceipt':receipt,'eth_blockNumber':hex(1002)}
            return d[method]
        with patch.object(p,'rpc',side_effect=fake),self.assertRaisesRegex(p.AdmissionError,'INSUFFICIENT_CONFIRMATIONS'):
            p.require_base_tx('https://test.invalid',TXID,TO)

    def test_quorum_disagreement_rejected(self):
        def fake(url,txid,destination):
            return {'tx_hash':TXID,'chain_id':p.CHAIN_ID,'from':SOURCE,'token':p.TOKEN,
                    'deposit_address':TO,'units':p.AMOUNT,'block_hash':BLOCK if url==p.BASE_RPCS[0] else '0x'+'c'*64,
                    'block_number':1000,'event_log_index':'0x1','token_code_sha256':p.TOKEN_CODE_SHA256,
                    'confirmations':30,'rpc':url}
        with patch.object(p,'require_base_tx',side_effect=fake),self.assertRaisesRegex(p.AdmissionError,'BASE_RPC_RECEIPT_OR_LOG_DISAGREEMENT'):
            p.verify_base_quorum(TXID,TO)

    def test_near_must_have_successful_actual_ft_transfer_receipt(self):
        def fake(url,method,params):
            if method=='status':return {'chain_id':'testnet','genesis_hash':p.NEAR_GENESIS}
            return {'transaction':{'hash':'H'*43,'signer_id':'relayer.testnet'},'status':{'SuccessValue':''},
                    'receipts_outcome':[{'outcome':{'status':{'SuccessValue':''}}}], 'receipts':[]}
        with patch.object(p,'rpc',side_effect=fake),self.assertRaisesRegex(p.AdmissionError,'NEAR_SIGNED_RECEIPT_DOES_NOT_PROVE_MINIMUM_DELIVERY'):
            p.verify_near_delivery('H'*43,'relayer.testnet',min_units=980000)

    def test_near_receipt_positive_synthetic(self):
        args=__import__('base64').b64encode(json.dumps({'receiver_id':p.RECEIVER,'amount':'990000'}).encode()).decode()
        receipt_id='R'*43
        block_hash='B'*43
        near={'transaction':{'hash':'H'*43,'signer_id':'relayer.testnet'},'status':{'SuccessValue':''},
              'receipts_outcome':[{'id':receipt_id,'block_hash':block_hash,'outcome':{'status':{'SuccessValue':''}}}],
              'receipts':[{'receipt_id':receipt_id,'receiver_id':p.USDC_NEAR_CONTRACT,'receipt':{'Action':{'actions':[{'FunctionCall':{'method_name':'ft_transfer','args':args}}]}}}]}
        def fake(url,method,params):
            if method=='status':return {'chain_id':'testnet','genesis_hash':p.NEAR_GENESIS}
            if method=='block':return {'header':{'hash':block_hash,'height':122}}
            return near
        with patch.object(p,'rpc',side_effect=fake):
            out=p.verify_near_delivery('H'*43,'relayer.testnet',min_units=980000)
            self.assertEqual(out['delivered_units'],990000)
            self.assertEqual(out['transfer_receipt']['block_height'],122)

    def test_near_receipt_without_outcome_cannot_claim_transfer(self):
        args=__import__('base64').b64encode(json.dumps({'receiver_id':p.RECEIVER,'amount':'990000'}).encode()).decode()
        near={'transaction':{'hash':'H'*43,'signer_id':'relayer.testnet'},'status':{'SuccessValue':''},
              'receipts_outcome':[{'id':'other','outcome':{'status':{'SuccessValue':''}}}],
              'receipts':[{'receipt_id':'unproven','receiver_id':p.USDC_NEAR_CONTRACT,
                           'receipt':{'Action':{'actions':[{'FunctionCall':{'method_name':'ft_transfer','args':args}}]}}}]}
        def fake(url,method,params):return {'chain_id':'testnet','genesis_hash':p.NEAR_GENESIS} if method=='status' else near
        with patch.object(p,'rpc',side_effect=fake),self.assertRaisesRegex(p.AdmissionError,'NEAR_TOKEN_TRANSFER_RECEIPT_OUTCOME_MISSING'):
            p.verify_near_delivery('H'*43,'relayer.testnet',min_units=980000)

    def test_insufficient_balance_delta_rejected(self):
        def fake(acct,hash):
            return {'units':100000000 if hash=='a'*43 else 100400000,
                    'block_hash':hash,'block_height':200 if hash=='a'*43 else 220}
        with patch.object(p,'near_ft_balance',side_effect=fake),self.assertRaisesRegex(p.AdmissionError,'NEAR_DESTINATION_BALANCE_INCREASE_BELOW_QUOTE_MINIMUM'):
            p.verify_delivery_balance('a'*43,'b'*43,980000)

    def test_balance_delta_positive_synthetic(self):
        def fake(acct,hash):
            return {'units':100000000 if hash=='a'*43 else 101000000,
                    'block_hash':hash,'block_height':200 if hash=='a'*43 else 220}
        with patch.object(p,'near_ft_balance',side_effect=fake):
            r=p.verify_delivery_balance('a'*43,'b'*43,980000)
            self.assertEqual(r['net_increase_units'],1_000_000)
            self.assertIn('not_sufficient',r['classification'])

    def test_no_network_funding_trade_claim_without_real_tx_inputs(self):
        self.assertFalse(p.is_tx_hash('fake'))
        self.assertFalse(p.is_tx_hash('0x'+'z'*64))

if __name__=='__main__':unittest.main()
