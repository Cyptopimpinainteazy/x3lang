"""Offline validation against genuine archived two-provider responses.

Tamper cases intentionally alter previously recorded real evidence. No mock
positive evidence, no RPC calls, no checkpoint authority.
"""
import copy
import json
import tempfile
import unittest
from pathlib import Path

from checkpoint_provider_crosscheck import CrosscheckError, verify_document, canon, sha

ROOT = Path(__file__).resolve().parents[2] / 'evidence' / 'near-checkpoint-review' / 'p27'
SAVED = ROOT / 'two-provider-crosscheck.json'


class ProviderCrosscheckTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.report = json.loads(SAVED.read_text())

    def verify(self, modified):
        return verify_document(modified, ROOT)

    def test_archived_two_provider_results_agree_but_never_authenticate(self):
        r = self.verify(self.report)
        self.assertTrue(r['rpc_providers_consistent'])
        self.assertEqual(r['historical_protocol_version'], 88)
        self.assertEqual(r['checkpoint_height'], 276392452)
        self.assertFalse(r['authenticated'])
        self.assertFalse(r['authorized_to_sign'])
        self.assertEqual(len(self.report['providers']), 2)
        for provider in self.report['providers']:
            self.assertEqual(len(provider['calls']), 5)

    def test_mutating_raw_report_breaks_integrity(self):
        mutated = copy.deepcopy(self.report)
        mutated['providers'][0]['calls']['candidate_block']['summary']['hash'] = 'forged'
        with self.assertRaisesRegex(CrosscheckError, 'checksum mismatch'):
            self.verify(mutated)

    def test_rpc_agreement_cannot_override_archived_checkpoint(self):
        mutated = copy.deepcopy(self.report)
        for provider in mutated['providers']:
            provider['calls']['candidate_block']['summary']['hash'] = '11111111111111111111111111111111'
        mutated['integrity_sha256'] = sha(canon({k:v for k,v in mutated.items() if k!='integrity_sha256'}))
        with self.assertRaisesRegex(CrosscheckError, 'historic candidate block mismatch'):
            self.verify(mutated)

    def test_rpc_agreement_cannot_self_authorize(self):
        mutated = copy.deepcopy(self.report)
        mutated['authenticated'] = True
        mutated['authorized_to_sign'] = True
        mutated['checkpoint_approved'] = True
        mutated['integrity_sha256'] = sha(canon({k:v for k,v in mutated.items() if k!='integrity_sha256'}))
        with self.assertRaisesRegex(CrosscheckError, 'cannot claim signing'):
            self.verify(mutated)

    def test_genesis_provenance_different_between_providers_rejected(self):
        mutated = copy.deepcopy(self.report)
        mutated['providers'][1]['calls']['status']['summary']['genesis_hash'] = '11111111111111111111111111111111'
        mutated['integrity_sha256'] = sha(canon({k:v for k,v in mutated.items() if k!='integrity_sha256'}))
        with self.assertRaisesRegex(CrosscheckError, 'providers disagree'):
            self.verify(mutated)

    def test_two_forged_matching_genesis_values_still_rejected(self):
        mutated = copy.deepcopy(self.report)
        for provider in mutated['providers']:
            provider['calls']['status']['summary']['genesis_hash'] = '11111111111111111111111111111111'
        mutated['integrity_sha256'] = sha(canon({k:v for k,v in mutated.items() if k!='integrity_sha256'}))
        with self.assertRaisesRegex(CrosscheckError, 'testnet chain identity'):
            self.verify(mutated)

    def test_protocol_upgrade_version_mismatch_rejected(self):
        mutated = copy.deepcopy(self.report)
        for provider in mutated['providers']:
            provider['calls']['historical_protocol']['summary']['protocol_version'] = 89
        mutated['integrity_sha256'] = sha(canon({k:v for k,v in mutated.items() if k!='integrity_sha256'}))
        with self.assertRaisesRegex(CrosscheckError, 'historical protocol identity/version'):
            self.verify(mutated)

    def test_invalid_raw_digest_rejected(self):
        mutated = copy.deepcopy(self.report)
        mutated['providers'][0]['calls']['status']['raw_response_sha256']='x'*64
        mutated['integrity_sha256'] = sha(canon({k:v for k,v in mutated.items() if k!='integrity_sha256'}))
        with self.assertRaisesRegex(CrosscheckError, 'missing RPC raw response digest'):
            self.verify(mutated)

    def test_modified_request_parameters_rejected(self):
        mutated = copy.deepcopy(self.report)
        mutated['providers'][0]['calls']['candidate_block']['params']={'block_id':123}
        mutated['integrity_sha256'] = sha(canon({k:v for k,v in mutated.items() if k!='integrity_sha256'}))
        with self.assertRaisesRegex(CrosscheckError, 'RPC request/provenance mismatch'):
            self.verify(mutated)

if __name__=='__main__':
    unittest.main()
