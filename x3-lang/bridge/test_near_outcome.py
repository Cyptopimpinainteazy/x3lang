"""Actual NEAR testnet receipt-proof fixture; no settlement authorization."""
import copy
import hashlib
import json
import unittest
from pathlib import Path

from near_merkle import decode_hash
from near_outcome import verify_receipt_outcome, OutcomeProofError

RAW = Path(__file__).parent / 'fixtures' / 'near_live_receipt_proof_20261105.json'
SHA = '0fb16971ff172bd30e6ada514a3c9ea688ac77815ea506f28be09b6b3512d5ab'
RECEIPT = '3f9FCzqC8k75AGob2UrCxbKVqADRyv5M5vGfoLEXbZyu'
EXECUTOR = 'league-v7.near-post.testnet'
HEAD = 'V7KcHcg16BVWWxhVbw2vQj87D3dJQwXchnU7DCVzo6V'
ROOT = decode_hash('6F2mBDUkzB8tRpETPpRTtH5zo1E5yGGpftfyNYbRVDVw')
OUTCOME_ROOT = '7mUR47bB4y7d7MbakknJGSdqmpCEDL5X47Y5LMq9BQnp'


class GenuineNEARReceiptTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        raw = RAW.read_bytes()
        assert hashlib.sha256(raw).hexdigest() == SHA, 'fixture modified'
        cls.proof = json.loads(raw)

    def verify(self, proof):
        return verify_receipt_outcome(proof, receipt_id=RECEIPT,
                                      receiver_id=EXECUTOR, light_client_head=HEAD,
                                      head_block_merkle_root=ROOT)

    def test_real_execution_outcome_proof_matches_header(self):
        result = self.verify(self.proof)
        self.assertEqual(result.block_outcome_root, OUTCOME_ROOT)
        self.assertEqual(result.block_hash, 'DSu44uX9GmwyGXzUhAVK1DWJYGRaUBbfT2tNRGQjEd5i')
        self.assertTrue(result.outcome_merkle_matches)
        self.assertTrue(result.block_merkle_matches)
        self.assertFalse(result.authenticated)
        self.assertFalse(result.authorized_to_sign)

    def test_real_receipt_mutations_all_fail_closed(self):
        cases = [
            ('gas', lambda x: x['result']['outcome_proof']['outcome'].__setitem__('gas_burnt', 1)),
            ('tokens', lambda x: x['result']['outcome_proof']['outcome'].__setitem__('tokens_burnt', '1')),
            ('recipient', lambda x: x['result']['outcome_proof']['outcome'].__setitem__('executor_id', 'attacker.testnet')),
            ('receipt', lambda x: x['result']['outcome_proof'].__setitem__('id', HEAD)),
            ('receipt ids', lambda x: x['result']['outcome_proof']['outcome'].__setitem__('receipt_ids', [])),
            ('logs', lambda x: x['result']['outcome_proof']['outcome'].__setitem__('logs', ['forged EVENT_JSON'])),
            ('status', lambda x: x['result']['outcome_proof']['outcome'].__setitem__('status', {'SuccessValue':'eA=='})),
            ('outcome path', lambda x: x['result']['outcome_proof']['proof'][0].__setitem__('direction', 'Left' if x['result']['outcome_proof']['proof'][0]['direction'] == 'Right' else 'Right')),
            ('outcome root path', lambda x: x['result']['outcome_root_proof'][0].__setitem__('direction', 'Left' if x['result']['outcome_root_proof'][0]['direction'] == 'Right' else 'Right')),
            ('outcome root', lambda x: x['result']['block_header_lite']['inner_lite'].__setitem__('outcome_root', HEAD)),
            ('block path', lambda x: x['result']['block_proof'][0].__setitem__('direction', 'Left' if x['result']['block_proof'][0]['direction'] == 'Right' else 'Right')),
            ('block hash', lambda x: x['result']['outcome_proof'].__setitem__('block_hash', HEAD)),
            ('bad base64', lambda x: x['result']['outcome_proof']['outcome'].__setitem__('status', {'SuccessValue':'!bad!'})),
        ]
        for label, change in cases:
            with self.subTest(label=label):
                mutated = copy.deepcopy(self.proof)
                change(mutated)
                with self.assertRaises((OutcomeProofError, ValueError)):
                    self.verify(mutated)

    def test_metadata_does_not_affect_partial_outcome_hash(self):
        # NEAR hashes PartialExecutionOutcome, which excludes metadata; changing
        # untrusted RPC metadata must never be treated as verified code identity.
        copy_proof = copy.deepcopy(self.proof)
        copy_proof['result']['outcome_proof']['outcome']['metadata']['version'] = 9999
        included = self.verify(copy_proof)
        self.assertTrue(included.outcome_merkle_matches)
        self.assertFalse(included.authenticated)

    def test_wrong_head_root_fails(self):
        with self.assertRaises((OutcomeProofError, ValueError)):
            verify_receipt_outcome(self.proof, receipt_id=RECEIPT, receiver_id=EXECUTOR,
                light_client_head=HEAD, head_block_merkle_root=decode_hash(HEAD))

    def test_wrong_network_head_not_blindly_accepted(self):
        with self.assertRaises((OutcomeProofError, ValueError)):
            verify_receipt_outcome(self.proof, receipt_id=RECEIPT, receiver_id=EXECUTOR,
                light_client_head='invalid!', head_block_merkle_root=ROOT)


if __name__ == '__main__':
    unittest.main()
