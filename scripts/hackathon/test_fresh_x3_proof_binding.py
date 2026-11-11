"""Fail-closed fresh .x3 owner nonce binding for vault finality verifier.

No account keys, wallet operations, or synthetic data presented as real chain evidence.
"""
from __future__ import annotations
from pathlib import Path
from tempfile import TemporaryDirectory
import unittest
import json
from vault_origin_two_chain_cli import verified_fresh_x3_source, execute, ProofRejected, LEGACY_SPENT_NONCE, REPO_ROOT

SOURCE=Path("docs/hackathon/candidates/base_vault_fresh_20261108.x3")
NONCE="x3_base_vault_20261108_0cf8a80f52f96a1629f04a23"
PROGRAM_SHA="73eaa27705c86872c3f34feeab634a2e2b7ea1f6b913390a1663a652813f4f03"

class FreshX3ProofBinding(unittest.TestCase):
    def test_real_typechecked_source_and_pinned_hash(self):
        source=verified_fresh_x3_source(SOURCE,NONCE,PROGRAM_SHA)
        self.assertEqual(source['nonce'],NONCE)
        self.assertEqual(source['program_sha256'],PROGRAM_SHA)
        self.assertEqual(source['origin_amount'],1_000_000)
        self.assertEqual(source['minimum_out'],980000)
        self.assertEqual(source['destination_account'],'x3lang.testnet')

    def test_historical_used_nonce_refused_at_proof_boundary(self):
        with self.assertRaisesRegex(ProofRejected,'HISTORICAL_ALREADY_USED'):
            verified_fresh_x3_source(SOURCE,LEGACY_SPENT_NONCE,PROGRAM_SHA)

    def test_wrong_new_nonce_rejected_even_for_valid_source(self):
        with self.assertRaisesRegex(ProofRejected,'X3_CANDIDATE_SOURCE_NOT_VERIFIED'):
            verified_fresh_x3_source(SOURCE,'x3_another_unused_candidate_98765',PROGRAM_SHA)

    def test_wrong_or_missing_owner_pinned_program_hash_refused(self):
        for val in ('', '0'*64, 'A'*64, None):
            with self.subTest(value=val), self.assertRaises(ProofRejected):
                verified_fresh_x3_source(SOURCE,NONCE,val)

    def test_source_outside_repo_or_symlink_refused(self):
        with TemporaryDirectory() as tmp:
            outside=Path(tmp)/'candidate.x3'
            outside.write_bytes((REPO_ROOT/SOURCE).read_bytes())
            with self.assertRaisesRegex(ProofRejected,'REPO_FILE_NO_SYMLINK'):
                verified_fresh_x3_source(outside,NONCE,PROGRAM_SHA)

    def test_substituted_bytes_and_malformed_source_are_refused(self):
        with TemporaryDirectory(dir=REPO_ROOT) as tmp:
            candidate=Path(tmp)/'candidate.x3'
            candidate.write_bytes((REPO_ROOT/SOURCE).read_bytes()+b'\n// replacement bytes\n')
            with self.assertRaisesRegex(ProofRejected,'X3_CANDIDATE_SOURCE_HASH_OR_NONCE_DRIFT'):
                verified_fresh_x3_source(candidate,NONCE,PROGRAM_SHA)
            candidate.write_bytes(b'')
            with self.assertRaisesRegex(ProofRejected,'X3_CANDIDATE_SOURCE_NOT_VERIFIED'):
                verified_fresh_x3_source(candidate,NONCE,PROGRAM_SHA)

    def test_old_cli_without_fresh_nonce_cannot_emit_any_evidence(self):
        with TemporaryDirectory() as tmp:
            root=Path(tmp)
            for name in ('base','near','quote'):
                (root/f'{name}.json').write_text(json.dumps({'classification':'SYNTHETIC_NOT_REAL'}))
            output=root/'evidence.json'
            self.assertEqual(execute(['--base-manifest',str(root/'base.json'),
             '--near-manifest',str(root/'near.json'),
             '--signed-quote-file',str(root/'quote.json'),
             '--output',str(output)]),2)
            self.assertFalse(output.exists())

if __name__=='__main__': unittest.main()
