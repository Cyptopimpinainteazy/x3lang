"""Real CLI + committed NEAR RPC fixture checks, no network or wallet access."""
from __future__ import annotations

import json
import os
import tempfile
import unittest
from pathlib import Path

from cryptographic_demo import DemoError, build_report, validate_pinned_fixture

ROOT = Path(__file__).resolve().parents[2]
X3C = ROOT / 'x3-lang/target/debug/x3c'


class JudgeCryptoDemoTests(unittest.TestCase):
    def test_compile_execute_real_x3_and_reject_real_chain_proof_tampering(self):
        if not X3C.is_file():
            self.skipTest('real x3c binary not built; run cargo build before test')
        with tempfile.TemporaryDirectory() as output:
            report = build_report(root=ROOT, x3c=X3C, output_dir=Path(output))
        self.assertEqual(report['schema'], 'x3.judge.crypto_demo.v1')
        self.assertEqual(report['compiler_vm']['source'], 'x3-lang/examples/timeout_refund_minimal.x3')
        self.assertEqual(report['compiler_vm']['check']['status'], 'passed')
        self.assertEqual(report['compiler_vm']['build']['status'], 'passed')
        self.assertEqual(report['compiler_vm']['execution']['status'], 'passed')
        self.assertEqual(report['compiler_vm']['execution']['mode'], 'local_dry_run')
        self.assertGreater(report['compiler_vm']['artifact_bytes'], 0)
        self.assertEqual(len(report['compiler_vm']['artifact_sha256']), 64)
        self.assertTrue(report['near']['receipt']['outcome_merkle_matches'])
        self.assertTrue(report['near']['receipt']['block_merkle_matches'])
        self.assertTrue(report['near']['validator_commitment']['matches'])
        self.assertTrue(report['near']['cross_provider']['consistent'])
        self.assertGreaterEqual(len(report['adversarial_tests']), 3)
        self.assertTrue(all(t['rejected'] for t in report['adversarial_tests']))
        self.assertFalse(report['release_authority']['trusted_checkpoint'])
        self.assertFalse(report['release_authority']['escrow_storage_authenticated'])
        self.assertFalse(report['release_authority']['cross_chain_causality_proven'])
        self.assertFalse(report['release_authority']['authorized_to_sign'])
        self.assertFalse(report['release_authority']['funds_moved'])

    def test_missing_production_compiler_is_hard_failure_not_mock(self):
        with tempfile.TemporaryDirectory() as output:
            with self.assertRaises(DemoError):
                build_report(root=ROOT, x3c=Path(output)/'fake_x3c', output_dir=Path(output))

    def test_modified_archived_real_receipt_is_rejected_before_decoding(self):
        src = ROOT / 'x3-lang/bridge/fixtures/near_live_receipt_proof_20261105.json'
        with tempfile.TemporaryDirectory() as output:
            altered = Path(output) / 'near_live_receipt_proof_20261105.json'
            altered.write_bytes(src.read_bytes().replace(b'league-v7.near-post.testnet', b'attack-v7.near-post.testnet'))
            with self.assertRaises(DemoError):
                validate_pinned_fixture(altered, '0fb16971ff172bd30e6ada514a3c9ea688ac77815ea506f28be09b6b3512d5ab')


if __name__ == '__main__':
    unittest.main()
