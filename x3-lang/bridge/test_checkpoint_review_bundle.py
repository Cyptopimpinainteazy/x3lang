"""Release-blocking tests for unsigned offline NEAR checkpoint review evidence."""
import json
import shutil
import tempfile
import unittest
from pathlib import Path

from checkpoint_review_bundle import (
    BundleError, build, verify, CANDIDATE, PREVIOUS, CHECKPOINT, VALIDATORS,
    MANIFEST, SOURCE_SHAS, assemble_documents, digest,
)
from near_authenticated import checkpoint_checksum

SOURCES = Path(__file__).resolve().parents[2] / 'evidence/near-checkpoint-review/p27'


class UnsignedReviewBundleTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.dir = Path(self.tmp.name)
        for file in (PREVIOUS, CANDIDATE):
            shutil.copyfile(SOURCES / file, self.dir / file)

    def test_build_verify_replay_is_deterministic_and_unapproved(self):
        first = build(self.dir)
        second = build(self.dir)
        self.assertEqual(first, second)
        self.assertEqual(first, verify(self.dir))
        self.assertEqual(first['status'], 'UNSIGNED_UNREVIEWED_RPC_CANDIDATE')
        self.assertFalse(first['authenticated'])
        self.assertFalse(first['authorized_to_sign'])
        self.assertEqual(first['checkpoint_height'], 276392452)
        manifest = json.loads((self.dir / MANIFEST).read_text())
        self.assertFalse(manifest['checkpoint_approved'])
        self.assertEqual(manifest['source_sha256'], SOURCE_SHAS)
        self.assertEqual(len(manifest['required_external_review']), 6)
        self.assertFalse(any('private_key' in x for x in manifest))
        checkpoint = json.loads((self.dir / CHECKPOINT).read_text())
        self.assertEqual(checkpoint_checksum(checkpoint), first['checkpoint_checksum'])
        self.assertIn('UNREVIEWED', checkpoint['provenance']['verified_by'])

    def test_independently_tampered_source_is_rejected(self):
        raw = bytearray((self.dir / PREVIOUS).read_bytes())
        raw[-4] ^= 1
        (self.dir / PREVIOUS).write_bytes(raw)
        with self.assertRaisesRegex(BundleError, 'checksum mismatch'):
            build(self.dir)
        self.assertFalse((self.dir / CHECKPOINT).exists())

    def test_tampered_current_validator_digest_is_rejected(self):
        build(self.dir)
        raw = json.loads((self.dir / VALIDATORS).read_text())
        raw['validators'][0]['stake'] = '1'
        (self.dir / VALIDATORS).write_text(json.dumps(raw))
        with self.assertRaisesRegex(BundleError, 'unexpected current-validators'):
            verify(self.dir)
        with self.assertRaisesRegex(BundleError, 'refusing overwrite'):
            build(self.dir)

    def test_tampered_unsigned_checkpoint_cannot_pass_validation(self):
        build(self.dir)
        raw = json.loads((self.dir / CHECKPOINT).read_text())
        raw['provenance']['verified_by'] = 'APPROVED by fake reviewer'
        (self.dir / CHECKPOINT).write_text(json.dumps(raw))
        with self.assertRaisesRegex(BundleError, 'unexpected checkpoint'):
            verify(self.dir)

    def test_changed_manifest_approval_flag_is_rejected(self):
        build(self.dir)
        raw = json.loads((self.dir / MANIFEST).read_text())
        raw['authenticated'] = True
        raw['authorized_to_sign'] = True
        (self.dir / MANIFEST).write_text(json.dumps(raw))
        with self.assertRaisesRegex(BundleError, 'unexpected review-manifest'):
            verify(self.dir)

    def test_archived_bytes_do_not_establish_network_trust(self):
        raw_p = (self.dir / PREVIOUS).read_bytes()
        raw_c = (self.dir / CANDIDATE).read_bytes()
        self.assertEqual(digest(raw_p), SOURCE_SHAS[PREVIOUS])
        self.assertEqual(digest(raw_c), SOURCE_SHAS[CANDIDATE])
        _, checkpoint, manifest = assemble_documents(raw_p, raw_c)
        self.assertEqual(checkpoint['network'], 'testnet')
        self.assertFalse(manifest['authenticated'])
        self.assertFalse(manifest['authorized_to_sign'])


if __name__ == '__main__':
    unittest.main()
