"""Read-only NEAR light-client proof ingestion; never an authorization."""
import unittest
from near_receipt_proof import ReceiptProofError, request_receipt_proof, get_candidate, authorize_receipt

RECEIPT = '1' * 32
HEAD = '1' * 31 + '2'
BLOCK = '1' * 31 + '3'
ACCOUNT = 'vault.testnet'


def sample():
    return {'jsonrpc':'2.0','id':'x3-near-light-proof','result':{
        'outcome_proof': {'id':RECEIPT,'block_hash':BLOCK,'proof':[],
            'outcome':{'executor_id':ACCOUNT,'logs':[], 'receipt_ids':[],
                       'gas_burnt':1,'tokens_burnt':'0','status':{'SuccessValue':''}}},
        'outcome_root_proof':[],
        'block_header_lite': {'prev_block_hash':BLOCK,'inner_rest_hash':BLOCK,
                              'inner_lite':{'height':100,'epoch_id':BLOCK,'next_epoch_id':BLOCK,
                                            'prev_state_root':BLOCK,'outcome_root':BLOCK,
                                            'timestamp':1000000000000000000,'next_bp_hash':BLOCK,
                                            'block_merkle_root':BLOCK}},
        'block_proof':[]}}


class ReceiptProofTests(unittest.TestCase):
    def test_requests_receipt_proof_and_never_authorizes(self):
        calls=[]
        def fetch(endpoint,payload):
            calls.append(payload)
            return sample()
        candidate=request_receipt_proof('https://rpc.example',receipt_id=RECEIPT,
                    receiver_id=ACCOUNT,light_client_head=HEAD,fetch=fetch)
        self.assertFalse(candidate.authenticated)
        self.assertFalse(candidate.authorized_to_sign)
        self.assertEqual(candidate.receipt_id,RECEIPT)
        self.assertEqual(calls[0]['method'],'light_client_proof')
        self.assertEqual(calls[0]['params']['light_client_head'],HEAD)
        self.assertEqual(calls[0]['params']['receiver_id'],ACCOUNT)
        with self.assertRaises(ReceiptProofError):
            authorize_receipt(candidate)

    def test_forged_executor_is_rejected(self):
        proof=sample(); proof['result']['outcome_proof']['outcome']['executor_id']='attacker.testnet'
        with self.assertRaises(ReceiptProofError):
            get_candidate(proof,receipt_id=RECEIPT,receiver_id=ACCOUNT,light_client_head=HEAD)

    def test_wrong_receipt_id_is_rejected(self):
        proof=sample(); proof['result']['outcome_proof']['id']=HEAD
        with self.assertRaises(ReceiptProofError):
            get_candidate(proof,receipt_id=RECEIPT,receiver_id=ACCOUNT,light_client_head=HEAD)

    def test_failed_receipt_is_rejected(self):
        proof=sample(); proof['result']['outcome_proof']['outcome']['status']={'Failure': {'ActionError': {}}}
        with self.assertRaises(ReceiptProofError):
            get_candidate(proof,receipt_id=RECEIPT,receiver_id=ACCOUNT,light_client_head=HEAD)

    def test_rpc_error_is_not_evidence(self):
        proof=sample(); proof.pop('result'); proof['error']={'code':-1,'message':'not found'}
        with self.assertRaises(ReceiptProofError):
            get_candidate(proof,receipt_id=RECEIPT,receiver_id=ACCOUNT,light_client_head=HEAD)

    def test_malformed_merkle_path_rejected(self):
        proof=sample(); proof['result']['block_proof']=[{'hash':BLOCK,'direction':'Sideways'}]
        with self.assertRaises(ReceiptProofError):
            get_candidate(proof,receipt_id=RECEIPT,receiver_id=ACCOUNT,light_client_head=HEAD)

    def test_unsupported_head_or_plain_rpc_status_cannot_authorize(self):
        with self.assertRaises(ReceiptProofError):
            request_receipt_proof('http://rpc.example',receipt_id=RECEIPT,
                                  receiver_id=ACCOUNT,light_client_head=HEAD,fetch=lambda *_: sample())
        with self.assertRaises(ReceiptProofError):
            get_candidate({'status':'FINAL'},receipt_id=RECEIPT,receiver_id=ACCOUNT,light_client_head=HEAD)

    def test_proof_bytes_are_bounded_and_hashes_are_base58(self):
        proof=sample(); proof['result']['outcome_proof']['proof']=[{'hash':'0'*43,'direction':'Left'}]
        with self.assertRaises(ReceiptProofError):
            get_candidate(proof,receipt_id=RECEIPT,receiver_id=ACCOUNT,light_client_head=HEAD)

if __name__ == '__main__': unittest.main()
