"""SYNTHETIC NEAR JSON-RPC unit fixtures, NEVER represented as public receipts."""
from __future__ import annotations
import base64,copy,json,unittest
from hashlib import sha256
from near_delivery_receipt_proof import NearExpected, NearProofRejected, verify_one, verify_two_providers, NEAR_GENESIS, TOKEN, DEST

HASH='F'*43
GUARD_HASH='A'*43
PRE_SOURCE='J'*43
AFTER_PRE_SOURCE='K'*43
BEFORE='B'*43
AFTER='C'*43
DELIVERY='D'*43
HEAD='E'*43
RECEIPT_ID='G'*43
NOW=1800000000
QUOTE='a'*64
PROGRAM='b'*64
OWNER=DEST
SIGNER='testrelayer.tidepool-sandbox.testnet'
GUARD='x3guardlaunch2026.testnet'
NONCE='x3_first_vault_nonce_001'
GUARD_WASM=b'actual-test-contract-binary-is-a-synthetic-placeholder'

def header(h,height,sec):
    return {'hash':h,'height':height,'timestamp_nanosec':str(sec*1_000_000_000)}

def setup():
    e=NearExpected(txhash=HASH,signer=SIGNER,recipient=DEST,token=TOKEN,minimum_units=900,
                   before_block_hash=BEFORE,after_block_hash=AFTER,guard_account=GUARD,
                   guard_code_sha256=sha256(GUARD_WASM).hexdigest(),guard_approval_block_hash=GUARD_HASH,
                   guard_pre_source_block_hash=PRE_SOURCE,owner=OWNER,nonce=NONCE,quote_sha256=QUOTE,program_sha256=PROGRAM,
                   origin_amount=1000,source_inclusion_unix_s=NOW,compiled_minimum_units=850,
                   refund_to='0x'+'12'*20,slippage_max_bps=100,quote_deadline_unix_s=NOW+1000)
    policy={'owner':OWNER,'nonce':NONCE,'revoked':False,'quote_sha256':QUOTE,'policy':{
        'program_sha256':PROGRAM,'amount_in':'1000','minimum_out':'900','expiry_ms':(NOW+1000)*1000,
        'refund_to':'0x'+'12'*20,'max_slippage_bps':100,
    }}
    near_transfer={'receipt_id':RECEIPT_ID,'receiver_id':TOKEN,'receipt':{'Action':{'actions':[
        {'FunctionCall':{'method_name':'ft_transfer','args':base64.b64encode(json.dumps({'receiver_id':DEST,'amount':'920'}).encode()).decode()}}
    ]}}}
    d={
      'status':{'chain_id':'testnet','genesis_hash':NEAR_GENESIS},
      'blocks':{GUARD_HASH:header(GUARD_HASH,1000,NOW-100),BEFORE:header(BEFORE,1001,NOW-10),
                PRE_SOURCE:header(PRE_SOURCE,1002,NOW-1),AFTER_PRE_SOURCE:header(AFTER_PRE_SOURCE,1003,NOW+1),
                DELIVERY:header(DELIVERY,1004,NOW+200),AFTER:header(AFTER,1005,NOW+250),HEAD:header(HEAD,1100,NOW+500)},
      'head':HEAD,
      'tx':{'final_execution_status':'FINAL','transaction':{'hash':HASH,'signer_id':SIGNER},
            'status':{'SuccessValue':''},'receipts_outcome':[
                {'id':RECEIPT_ID,'outcome':{'status':{'SuccessValue':''}},'block_hash':DELIVERY}],
            'receipts':[near_transfer]},
      'balance':{BEFORE:10000,AFTER:10920},
      'policy':policy,'pre_source_policy':copy.deepcopy(policy),'guard_code':GUARD_WASM,
    }
    return e,d

class MockNear:
    def __init__(self,d): self.d=copy.deepcopy(d)
    def __call__(self,method,param):
        d=self.d
        if method=='status':return d['status']
        if method=='EXPERIMENTAL_tx_status':return d['tx']
        if method=='block':
            ident=d['head'] if param.get('finality')=='final' else param.get('block_id')
            if isinstance(ident,int):
                ids=[h for h in d['blocks'].values() if h['height']==ident]
                if not ids: raise ValueError('unknown block')
                return {'header':ids[0]}
            return {'header':d['blocks'][ident]}
        if method=='query':
            kind=param.get('request_type')
            if kind=='view_code':return {'block_hash':param['block_id'],'code_base64':base64.b64encode(d['guard_code']).decode()}
            if kind=='call_function':
                if param['method_name']=='get_policy':
                    at=param['block_id']
                    value=d['pre_source_policy'] if at==PRE_SOURCE else d['policy']
                    return {'block_hash':at,'result':list(json.dumps(value).encode())}
                if param['method_name']=='ft_balance_of':
                    h=param['block_id']
                    amount=d['balance'][h]
                    return {'block_hash':h,'block_height':d['blocks'][h]['height'],'result':list(json.dumps(str(amount)).encode())}
        raise AssertionError('Unknown RPC request '+str((method,param)))

class NearProofTests(unittest.TestCase):
    def setUp(self):
        self.e,self.d=setup()

    def test_two_provider_synthetic_fixture_does_not_claim_atomicity(self):
        p=verify_two_providers((('https://near-a.example',MockNear(self.d)),('https://near-b.example',MockNear(self.d))),self.e)
        self.assertEqual(p['provider_count'],2)
        self.assertEqual(p['provider_hosts'],['near-a.example','near-b.example'])
        self.assertFalse(p['cryptographic_validator_finality_proven'])
        self.assertFalse(p['relayer_causality_proven'])
        self.assertEqual(p['observations'][0]['balance_delta'],920)
        self.assertEqual(p['observations'][0]['ft_receipt']['amount'],920)

    def test_wrong_genesis_rejected(self):
        d=copy.deepcopy(self.d);d['status']['genesis_hash']='wrong'
        with self.assertRaisesRegex(NearProofRejected,'NEAR_GENESIS_MISMATCH'):
            verify_one(MockNear(d),self.e)

    def test_optimistic_or_failed_tx_not_final(self):
        for change in ('optimistic','failed'):
            d=copy.deepcopy(self.d)
            if change=='optimistic':d['tx']['final_execution_status']='EXECUTED_OPTIMISTIC'
            else:d['tx']['receipts_outcome'][0]['outcome']['status']={'Failure':{'ActionError':{}}}
            with self.subTest(change=change),self.assertRaises(NearProofRejected):
                verify_one(MockNear(d),self.e)

    def test_wrong_recipient_and_duplicate_ft_transfer_rejected(self):
        for mode in ('redirect','duplicate','amount_too_low'):
            d=copy.deepcopy(self.d)
            r=d['tx']['receipts'][0]
            call=r['receipt']['Action']['actions'][0]['FunctionCall']
            terms=json.loads(base64.b64decode(call['args']))
            if mode=='redirect':terms['receiver_id']='attacker.testnet'
            if mode=='amount_too_low':terms['amount']='3'
            call['args']=base64.b64encode(json.dumps(terms).encode()).decode()
            if mode=='duplicate':
                extra=copy.deepcopy(r);extra['receipt_id']='H'*43
                d['tx']['receipts'].append(extra)
                d['tx']['receipts_outcome'].append({'id':extra['receipt_id'],'outcome':{'status':{'SuccessValue':''}},'block_hash':DELIVERY})
            with self.subTest(mode=mode),self.assertRaises(NearProofRejected):verify_one(MockNear(d),self.e)

    def test_balance_only_without_real_receipt_cannot_prove_settlement(self):
        d=copy.deepcopy(self.d);d['tx']['receipts']=[]
        with self.assertRaisesRegex(NearProofRejected,'NEAR_EXPECTED_FT_DELIVERY_MISSING_OR_DUPLICATE'):
            verify_one(MockNear(d),self.e)

    def test_wrong_balance_delta_or_noncanonical_snapshots(self):
        for mode in ('delta','height'):
            d=copy.deepcopy(self.d)
            if mode=='delta':d['balance'][AFTER]=10000
            else:d['blocks'][AFTER]['height']=999
            with self.subTest(mode=mode),self.assertRaises(NearProofRejected):verify_one(MockNear(d),self.e)

    def test_guard_approval_after_source_cannot_be_passed(self):
        d=copy.deepcopy(self.d);d['blocks'][GUARD_HASH]['timestamp_nanosec']=str((NOW+2)*1_000_000_000)
        with self.assertRaisesRegex(NearProofRejected,'GUARD_APPROVED_AFTER_BASE_PAYMENT'):
            verify_one(MockNear(d),self.e)

    def test_guard_wrong_code_policy_nonce_quote_or_program_rejected(self):
        for mode in ('code','nonce','quote','program','expiry','minimum','refund','slippage'):
            d=copy.deepcopy(self.d)
            if mode=='code':d['guard_code']=b'wrong'
            elif mode=='nonce':d['policy']['nonce']='attacker'
            elif mode=='quote':d['policy']['quote_sha256']='0'*64
            elif mode=='program':d['policy']['policy']['program_sha256']='0'*64
            elif mode=='expiry':d['policy']['policy']['expiry_ms']=0
            elif mode=='minimum':d['policy']['policy']['minimum_out']='100'
            elif mode=='refund':d['policy']['policy']['refund_to']='0x'+'34'*20
            elif mode=='slippage':d['policy']['policy']['max_slippage_bps']=500
            with self.subTest(mode=mode),self.assertRaises(NearProofRejected):verify_one(MockNear(d),self.e)

    def test_guard_revoked_or_altered_before_base_transfer_is_rejected(self):
        # A valid old approval does not imply the owner kept it active until
        # the Base payment. The pre-payment finalized NEAR state must agree.
        for mode in ('revoked', 'different_quote', 'different_program'):
            d=copy.deepcopy(self.d)
            if mode=='revoked': d['pre_source_policy']['revoked']=True
            elif mode=='different_quote':d['pre_source_policy']['quote_sha256']='0'*64
            else:d['pre_source_policy']['policy']['program_sha256']='0'*64
            with self.subTest(mode=mode),self.assertRaisesRegex(NearProofRejected,'GUARD_'):
                verify_one(MockNear(d),self.e)

    def test_guard_pre_source_requires_closest_canonical_checkpoint(self):
        for mode in ('old_snapshot', 'noncanonical', 'successor_not_newer', 'pre_snapshot_after_base'):
            d=copy.deepcopy(self.d)
            if mode=='old_snapshot':d['blocks'][AFTER_PRE_SOURCE]['timestamp_nanosec']=str((NOW-1)*1_000_000_000)
            elif mode=='noncanonical':d['blocks'][PRE_SOURCE]['hash']='L'*43
            elif mode=='successor_not_newer':d['blocks'][AFTER_PRE_SOURCE]['timestamp_nanosec']=str(NOW*1_000_000_000)
            elif mode=='pre_snapshot_after_base':d['blocks'][PRE_SOURCE]['timestamp_nanosec']=str((NOW+2)*1_000_000_000)
            with self.subTest(mode=mode),self.assertRaises(NearProofRejected):
                verify_one(MockNear(d),self.e)

    def test_near_provider_disagreement_rejected(self):
        b=copy.deepcopy(self.d);b['balance'][AFTER]+=1
        with self.assertRaisesRegex(NearProofRejected,'NEAR_FINALIZED_RPC_EVIDENCE_DISAGREEMENT'):
            verify_two_providers((('https://near-a.example',MockNear(self.d)),('https://near-b.example',MockNear(b))),self.e)

    def test_near_provider_url_must_not_embed_credentials_or_fragment(self):
        for bad in ('https://near-a.example/?token=secret', 'https://near-a.example/#secret'):
            with self.subTest(url=bad),self.assertRaises(NearProofRejected):
                verify_two_providers(((bad,MockNear(self.d)),('https://near-b.example',MockNear(self.d))),self.e)

    def test_near_single_provider_or_duplicate_domain_rejected(self):
        with self.assertRaisesRegex(NearProofRejected,'NEAR_RPC_PROVIDERS_NOT_INDEPENDENT'):
            verify_two_providers((('https://same.example',MockNear(self.d)),('https://same.example',MockNear(self.d))),self.e)

if __name__=='__main__':unittest.main()
