"""SYNTHETIC RPC fixtures ONLY. No fabricated real transactions.

Positive fixture tests demonstrate validator logic, not deployed Base proof.
The separate historical EOA negative test uses the real public tx hash but
minimal simulated RPC fields to prove correct type discrimination.
"""
from __future__ import annotations
import copy
from hashlib import sha256
import unittest

from keccak_min import keccak256
from vault_origin_base_proof import (
    Expected, ProofRejected, PAYMENT_TOPIC, TRANSFER_TOPIC, PAYMENT_SELECTOR,
    address, encoded_address, uint256, hash32, encode_static_call, decode_payment_calldata,
    verify_one, verify_two_providers,
)

HISTORICAL_EOA_TX='0x46c21dc305a9271c4e74c6d0734fb50f847a1e53d3cad87976133d66fd6728d9'
V='0x'+'11'*20
O='0x'+'22'*20
A='0x'+'33'*20
T='0x'+'44'*20
D='0x'+'55'*20
R='0x'+'66'*20
TX='0x'+'12'*32
BH='0x'+'f0'*32
FINAL_HASH='0x'+'bb'*32
NONCE='0x'+'bc'*32
QUOTE='ac'*32
PROGRAM='bd'*32
TIMESTAMP=1790000000
DEADLINE=TIMESTAMP+100
EXPIRES=TIMESTAMP+300
AMOUNT=100_000
VAULT_CODE=b'\x60\x01\x60\x02' # testing-only synthetic bytecode
TOKEN_CODE=b'\x60\x03\x60\x04'
DIGEST='0x'+'e5'*32
SIG=b'\x11'*64+b'\x1b'

def calldata(*,nonce=NONCE,recipient=D,amount=AMOUNT,deadline=DEADLINE,signature=SIG):
    args=(bytes.fromhex(nonce[2:])+encoded_address(recipient)+uint256(amount)
          +uint256(deadline)+uint256(160)+uint256(len(signature))
          +signature+b'\0'*((-len(signature))%32))
    return '0x'+(PAYMENT_SELECTOR+args).hex()

def log(addr, topics, data, index):
    return dict(address=addr,topics=topics,data='0x'+data.hex(),logIndex=hex(index),
                transactionHash=TX,blockHash=BH,removed=False)

def fixture():
    e=Expected(V,O,A,T,D,QUOTE,PROGRAM,sha256(TOKEN_CODE).hexdigest(),sha256(VAULT_CODE).hexdigest(),
               NONCE,AMOUNT,DEADLINE,EXPIRES,AMOUNT,AMOUNT)
    tx=dict(hash=TX,**{'from':R},to=V,value='0x0',input=calldata(),blockHash=BH,blockNumber='0x64')
    receipt=dict(transactionHash=TX,**{'from':R},to=V,status='0x1',blockHash=BH,blockNumber='0x64',
                 logs=[
                     log(V,[PAYMENT_TOPIC,NONCE,'0x'+encoded_address(D).hex()],uint256(AMOUNT)+bytes.fromhex(DIGEST[2:]),0),
                     log(T,[TRANSFER_TOPIC,'0x'+encoded_address(V).hex(),'0x'+encoded_address(D).hex()],uint256(AMOUNT),1),
                 ])
    return e,{
        'eth_chainId':'0x14a34',
        'eth_getTransactionByHash':tx,
        'eth_getTransactionReceipt':receipt,
        'eth_getBlockByNumber':{'0x64':dict(number='0x64',hash=BH,timestamp=hex(TIMESTAMP)),
                                'finalized':dict(number='0x72',hash=FINAL_HASH)},
        'eth_blockNumber':'0x76',
        'eth_getCode':{address(V):'0x'+VAULT_CODE.hex(),address(T):'0x'+TOKEN_CODE.hex()},
        'eth_call':{},
    }

def inject_calls(e, data):
    m=data['eth_call']
    for name,val in [('owner()',O),('asset()',T),('agent()',A),('recipient()',D)]:
        m[encode_static_call(name)]= '0x'+encoded_address(val).hex()
    for name,val in [('quoteHash()',QUOTE),('programHash()',PROGRAM)]:
        m[encode_static_call(name)]='0x'+val
    m[encode_static_call('tokenCodeHash()')]='0x'+keccak256(TOKEN_CODE).hex()
    for name,val in [('maxPerPayment()',AMOUNT),('maxTotal()',AMOUNT),('expiresAt()',EXPIRES),('spent()',AMOUNT)]:
        m[encode_static_call(name)]='0x'+uint256(val).hex()
    m[encode_static_call('usedNonces(bytes32)',bytes.fromhex(NONCE[2:]))]='0x'+uint256(1).hex()
    m[encode_static_call('paymentDigest(bytes32,address,uint256,uint256)',bytes.fromhex(NONCE[2:])+encoded_address(D)+uint256(AMOUNT)+uint256(DEADLINE))]=DIGEST
    return m

class FakeRPC:
    """Clearly synthetic, deterministic mock JSON-RPC *responses*, never receipt evidence."""
    def __init__(self, data): self.data=copy.deepcopy(data)
    def __call__(self, method, params):
        d=self.data
        if method in ('eth_chainId','eth_blockNumber'):
            return d[method]
        if method in ('eth_getTransactionByHash','eth_getTransactionReceipt'):
            return d[method]
        if method=='eth_getBlockByNumber':
            return d[method][params[0]]
        if method=='eth_getCode':
            return d[method][address(params[0])]
        if method=='eth_call':
            return d[method][params[0]['data']]
        raise AssertionError('Unexpected JSON RPC method '+str(method))

class StrictVaultSourceProofTests(unittest.TestCase):
    def setUp(self):
        self.expected, self.data=fixture()
        inject_calls(self.expected,self.data)
        self.rpc=FakeRPC(self.data)

    def test_keccak_known_erc20_selector_and_event(self):
        self.assertEqual(keccak256(b'transfer(address,uint256)').hex()[:8],'a9059cbb')
        self.assertEqual(TRANSFER_TOPIC,'0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef')
        self.assertEqual(len(PAYMENT_TOPIC),66)

    def test_synthetic_valid_receipt_is_not_a_crosschain_certificate(self):
        p=verify_two_providers((('https://provider-one.example',FakeRPC(self.data)),('https://provider-two.example',FakeRPC(self.data))),self.expected,TX)
        self.assertEqual(p['independent_provider_count'],2)
        self.assertEqual(p['provider_hosts'],['provider-one.example','provider-two.example'])
        self.assertEqual(p['classification'],'READ_ONLY_BASE_VAULT_ORIGIN_FINALIZED_EVIDENCE_NOT_CROSSCHAIN_PROOF')
        self.assertFalse(p['destination_near_verified'])
        self.assertFalse(p['relayer_atomicity_proven'])
        self.assertFalse(p['broadcast'])

    def test_old_real_historical_eoa_transfer_cannot_be_misclassified(self):
        # Actual old hash; payload below is intentionally synthetic/minimal.
        self.data['eth_getTransactionByHash'].update(hash=HISTORICAL_EOA_TX,to=T,
                input='0xa9059cbb'+encoded_address(D).hex()+uint256(AMOUNT).hex())
        self.data['eth_getTransactionReceipt']['transactionHash']=HISTORICAL_EOA_TX
        with self.assertRaisesRegex(ProofRejected,'NOT_VAULT_ORIGIN_TX_TO'):
            verify_one(FakeRPC(self.data),self.expected,HISTORICAL_EOA_TX)

    def test_incorrect_transaction_selector_rejected(self):
        self.data['eth_getTransactionByHash']['input']='0xa9059cbb'+calldata()[10:]
        with self.assertRaisesRegex(ProofRejected,'NOT_VAULT_EXECUTE_PAYMENT_SELECTOR'):
            verify_one(FakeRPC(self.data),self.expected,TX)

    def test_bad_agent_signed_payment_nonce_recipient_amount_and_deadline(self):
        for kw,reason in [({'nonce':'0x'+'88'*32},'SIGNED_ORDER_NONCE_MISMATCH'),
                          ({'recipient':O},'SIGNED_ORDER_DEPOSIT_REDIRECT'),
                          ({'amount':AMOUNT+1},'SIGNED_ORDER_AMOUNT_MISMATCH'),
                          ({'deadline':DEADLINE+1},'SIGNED_ORDER_DEADLINE_MISMATCH')]:
            with self.subTest(kw=kw):
                data=copy.deepcopy(self.data)
                data['eth_getTransactionByHash']['input']=calldata(**kw)
                with self.assertRaisesRegex(ProofRejected,reason):
                    verify_one(FakeRPC(data),self.expected,TX)

    def test_event_tampering_or_duplicate_or_missing_token_transfer_refused(self):
        for mode in ('missing_payment','duplicate_payment','wrong_amount','redirect','missing_transfer'):
            data=copy.deepcopy(self.data)
            logs=data['eth_getTransactionReceipt']['logs']
            if mode=='missing_payment': logs.pop(0)
            elif mode=='duplicate_payment':
                duplicate=copy.deepcopy(logs[0]);duplicate['logIndex']='0x3';logs.append(duplicate)
            elif mode=='wrong_amount': logs[0]['data']='0x'+(uint256(AMOUNT+1)+bytes.fromhex(DIGEST[2:])).hex()
            elif mode=='redirect': logs[1]['topics'][2]='0x'+encoded_address(O).hex()
            elif mode=='missing_transfer': logs.pop(1)
            with self.subTest(mode=mode),self.assertRaises(ProofRejected):
                verify_one(FakeRPC(data),self.expected,TX)

    def test_token_transfer_cannot_precede_vault_event(self):
        data=copy.deepcopy(self.data)
        data['eth_getTransactionReceipt']['logs'][0]['logIndex']='0x2'
        data['eth_getTransactionReceipt']['logs'][1]['logIndex']='0x1'
        with self.assertRaisesRegex(ProofRejected,'VAULT_EVENT_MUST_PRECEDE_TOKEN_PAYOUT'):
            verify_one(FakeRPC(data),self.expected,TX)

    def test_no_rpc_write_methods_allowed(self):
        from vault_origin_base_proof import _https_rpc
        rpc=_https_rpc('https://rpc.example')
        with self.assertRaisesRegex(ProofRejected,'NON_READ_ONLY_RPC_METHOD_FORBIDDEN'):
            rpc('eth_sendRawTransaction',['0xdeadbeef'])

    def test_invalid_cli_manifest_never_emits_evidence(self):
        import tempfile
        import json
        from pathlib import Path
        from vault_origin_base_proof import run_readonly_cli
        with tempfile.TemporaryDirectory() as tmp:
            manifest=Path(tmp)/'manifest.json';out=Path(tmp)/'evidence.json'
            manifest.write_text(json.dumps({'tx_hash':HISTORICAL_EOA_TX,'rpc_urls':[]}))
            self.assertEqual(run_readonly_cli(['--review-manifest',str(manifest),'--output',str(out)]),2)
            self.assertFalse(out.exists())

    def test_token_or_vault_codehash_mismatch_refused(self):
        for target in (T,V):
            data=copy.deepcopy(self.data)
            data['eth_getCode'][address(target)]='0x60016008'
            with self.subTest(target=target),self.assertRaises(ProofRejected):
                verify_one(FakeRPC(data),self.expected,TX)

    def test_wrong_constructor_immutable_or_digest_refused(self):
        for signature in ('owner()','quoteHash()','paymentDigest(bytes32,address,uint256,uint256)'):
            data=copy.deepcopy(self.data)
            for k in list(data['eth_call']):
                if k.startswith(encode_static_call(signature)[:10]):
                    data['eth_call'][k]='0x'+'00'*32
            with self.subTest(signature=signature),self.assertRaises(ProofRejected):
                verify_one(FakeRPC(data),self.expected,TX)

    def test_finality_and_chain_id_fail_closed(self):
        for mode in ('wrong_chain','nonfinal','reorg','low_confirmations','reverted'):
            data=copy.deepcopy(self.data)
            if mode=='wrong_chain': data['eth_chainId']='0x1'
            elif mode=='nonfinal': data['eth_getBlockByNumber']['finalized']['number']='0x63'
            elif mode=='reorg': data['eth_getBlockByNumber']['0x64']['hash']='0x'+'aa'*32
            elif mode=='low_confirmations': data['eth_blockNumber']='0x65'
            elif mode=='reverted': data['eth_getTransactionReceipt']['status']='0x0'
            with self.subTest(mode=mode),self.assertRaises(ProofRejected):
                verify_one(FakeRPC(data),self.expected,TX)

    def test_rpc_disagreement_or_one_provider_only_rejected(self):
        data=copy.deepcopy(self.data)
        data['eth_getTransactionReceipt']['blockHash']='0x'+'a1'*32
        with self.assertRaises(ProofRejected):
            verify_two_providers((('https://one.example',FakeRPC(self.data)),('https://two.example',FakeRPC(data))),self.expected,TX)
        with self.assertRaises(ProofRejected):
            verify_two_providers((('https://one.example',FakeRPC(self.data)),('https://one.example',FakeRPC(self.data))),self.expected,TX)

    def test_provider_url_must_not_embed_credentials_or_fragment(self):
        for bad in ('https://one.example/?token=secret', 'https://one.example/#secret'):
            with self.subTest(url=bad),self.assertRaises(ProofRejected):
                verify_two_providers(((bad,FakeRPC(self.data)),('https://two.example',FakeRPC(self.data))),self.expected,TX)

    def test_signature_padding_and_offsets_cannot_be_modified(self):
        for value in (calldata(signature=SIG[:-1]),calldata()+'00',calldata()[:4+160]+'00'*32):
            with self.subTest(size=len(value)),self.assertRaises(ProofRejected):
                decode_payment_calldata(value)

    def test_no_near_evidence_flag_cannot_be_overridden(self):
        result=verify_two_providers((('https://one.example',FakeRPC(self.data)),('https://two.example',FakeRPC(self.data))),self.expected,TX)
        self.assertFalse(result['destination_near_verified'])
        self.assertFalse(result['signed'])

if __name__=='__main__':unittest.main()
