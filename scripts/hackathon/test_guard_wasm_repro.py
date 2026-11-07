"""Offline integrity tests, deliberately synthetic only for mutation rejection."""
import hashlib
import json
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from verify_guard_wasm_bytes import IntegrityError, b58encode, verify_bytes


def fixture():
    synthetic = b'\x00asm\x01\x00\x00\x00not-an-actual-contract'
    digest = hashlib.sha256(synthetic).digest()
    manifest = {
        'schema':'x3.guard.testnet.manifest.v1',
        'chain_id':'testnet',
        'account_id':'x3lang-guard-20261106.testnet',
        'wasm_bytes':len(synthetic),
        'wasm_sha256':digest.hex(),
        'code_hash':b58encode(digest),
    }
    manifest['record_hash']=hashlib.sha256(json.dumps(manifest,sort_keys=True,separators=(',',':'),ensure_ascii=False).encode()).hexdigest()
    return synthetic, manifest


class ReproGuardTests(unittest.TestCase):
    def test_internal_manifest_hashes_are_consistent_only_if_byte_equal(self):
        raw, manifest=fixture()
        self.assertEqual(verify_bytes(manifest, raw, expected_sha=manifest['wasm_sha256'])['wasm_bytes'],len(raw))
        with self.assertRaises(IntegrityError):
            verify_bytes(manifest, raw+b'!',expected_sha=manifest['wasm_sha256'])

    def test_rejects_corrupted_manifest_record(self):
        raw, manifest=fixture()
        manifest['wasm_bytes']+=1
        with self.assertRaisesRegex(IntegrityError,'manifest'):
            verify_bytes(manifest,raw,expected_sha=manifest['wasm_sha256'])

    def test_rejects_changed_code_hash_even_when_manifest_rehashed(self):
        raw, manifest=fixture()
        manifest['code_hash']='11111111111111111111111111111111'
        manifest['record_hash']=hashlib.sha256(json.dumps({k:v for k,v in manifest.items() if k!='record_hash'},sort_keys=True,separators=(',',':'),ensure_ascii=False).encode()).hexdigest()
        with self.assertRaisesRegex(IntegrityError,'code_hash'):
            verify_bytes(manifest,raw,expected_sha=manifest['wasm_sha256'])

    def test_rejects_wrong_pinned_deployment_sha_even_when_manifest_matches(self):
        raw, manifest=fixture()
        with self.assertRaisesRegex(IntegrityError,'pinned'):
            verify_bytes(manifest,raw,expected_sha='0'*64)


if __name__=='__main__':unittest.main()
