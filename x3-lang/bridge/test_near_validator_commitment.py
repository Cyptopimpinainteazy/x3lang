"""Real NEAR testnet validator set commitment; not checkpoint finality."""
import copy
import hashlib
import json
import unittest
from pathlib import Path

from near_validator_commitment import compute_next_bps_hash, check_next_bps_commitment

RAW = Path(__file__).parent / 'fixtures' / 'near_live_light_client_update_20261105.json'
SHA = 'c47f0c381dbd9a37ac432aa4f77cf9d47ea8ccc452bfe4ff2a67b868953caed7'
EXPECTED = '4nzeJKGgHtgtESYKxfNh7Qq38WuYxCZ4zbVhjQHUYvez'

class LiveNextBPCommitmentTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        raw = RAW.read_bytes()
        assert hashlib.sha256(raw).hexdigest() == SHA
        cls.response = json.loads(raw)['result']

    def test_live_next_bp_hash_matches_header(self):
        evidence = check_next_bps_commitment(self.response)
        self.assertEqual(evidence.next_bp_hash, EXPECTED)
        self.assertEqual(evidence.validator_count, 20)
        self.assertFalse(evidence.authenticated)
        self.assertFalse(evidence.authorized_to_sign)
        self.assertEqual(compute_next_bps_hash(self.response['next_bps']), EXPECTED)

    def test_validator_corruptions_rejected(self):
        cases = [
            ('stake', lambda x: x['next_bps'][0].__setitem__('stake','1')),
            ('public key', lambda x: x['next_bps'][0].__setitem__('public_key', x['next_bps'][1]['public_key'])),
            ('account', lambda x: x['next_bps'][0].__setitem__('account_id','imposter.testnet')),
            ('order', lambda x: x['next_bps'].reverse()),
            ('missing', lambda x: x['next_bps'].pop()),
            ('header', lambda x: x['inner_lite'].__setitem__('next_bp_hash', x['inner_lite']['epoch_id'])),
            ('wrong key algorithm', lambda x: x['next_bps'][0].__setitem__('public_key','secp256k1:'+x['next_bps'][0]['public_key'].split(':')[1])),
            ('unknown struct', lambda x: x['next_bps'][0].__setitem__('validator_stake_struct_version','V9')),
            ('bad amount', lambda x: x['next_bps'][0].__setitem__('stake', '-1')),
            ('dupe validator', lambda x: x['next_bps'].__setitem__(1,x['next_bps'][0])),
        ]
        for label, change in cases:
            with self.subTest(label=label):
                v=copy.deepcopy(self.response)
                change(v)
                with self.assertRaises(ValueError): check_next_bps_commitment(v)

    def test_does_not_trust_self_reported_validators(self):
        x = self.response['next_bps']
        self.assertEqual(compute_next_bps_hash(x), EXPECTED)
        # A malicious endpoint can replace the entire list AND the header.
        # Commitment agreement alone never authorizes a real chain checkpoint.
        fake=copy.deepcopy(self.response)
        fake['next_bps'][0]['stake']='1'
        fake['inner_lite']['next_bp_hash']=compute_next_bps_hash(fake['next_bps'])
        v=check_next_bps_commitment(fake)
        self.assertFalse(v.authenticated)
        self.assertFalse(v.authorized_to_sign)

if __name__=='__main__': unittest.main()
