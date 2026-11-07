"""Boundary tests: Guard authorization MUST precede keystore and broadcast."""
from __future__ import annotations
from pathlib import Path
import importlib.util
import sys
import tempfile
import subprocess
import unittest
from unittest.mock import patch

ROOT=Path(__file__).resolve().parents[2]
sys.path.insert(0,str(ROOT/'x3-lang'))
sys.path.insert(0,str(Path(__file__).resolve().parent))
p=Path(__file__).with_name('base_guarded_broadcast.py')
spec=importlib.util.spec_from_file_location('guarded_sender',p)
m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m)

class TestSender(unittest.TestCase):
    def setUp(self):
        self.tmp=tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.saved=m.KEYSTORE_DIR
        m.KEYSTORE_DIR=Path(self.tmp.name)
        self.addCleanup(setattr,m,'KEYSTORE_DIR',self.saved)
        self.args={'quote_file':Path(self.tmp.name)/'quote.json',
                   'guard_account':'x3-tidepool-guard.testnet','guard_sha':'a'*64,
                   'owner_nonce':'testnonce1','program_sha':'b'*64,
                   'approved_quote_sha':'c'*64,'execute':True}

    def test_wrong_source_hash_prevents_any_quote_or_chain_lookup(self):
        with patch.object(m,'compile_testnet_intent',return_value={'program_sha256':'a'*64}), \
             patch.object(m.admission,'check_quote') as quote, \
             patch.object(m.admission,'get_near_policy') as near:
            with self.assertRaisesRegex(m.admission.AdmissionError,'X3_SOURCE_HASH_MISMATCH'):
                m.checked_admission(self.args['quote_file'],self.args['guard_account'],self.args['guard_sha'],self.args['owner_nonce'],'b'*64)
            quote.assert_not_called();near.assert_not_called()

    def test_compiled_nonce_mismatch_stops_before_signed_quote_or_wallet(self):
        source={'program_sha256':'b'*64,'nonce':'only_allowed_nonce'}
        with patch.object(m,'compile_testnet_intent',return_value=source), \
             patch.object(m.admission,'check_quote') as quote, \
             patch.object(m.admission,'get_near_policy') as near, \
             patch.object(m.subprocess,'run') as cast:
            with self.assertRaisesRegex(m.admission.AdmissionError,'X3_COMPILED_NONCE_MISMATCH'):
                m.checked_admission(self.args['quote_file'],self.args['guard_account'],self.args['guard_sha'],self.args['owner_nonce'],'b'*64)
            quote.assert_not_called();near.assert_not_called();cast.assert_not_called()

    def test_invalid_guard_fails_without_cast_or_key(self):
        with patch.object(m,'checked_admission',side_effect=m.admission.AdmissionError('GUARD_DENIED')) as gate,\
             patch.object(m.subprocess,'run') as cast,\
             patch.object(m,'assert_credentials') as secrets:
            with self.assertRaisesRegex(m.admission.AdmissionError,'GUARD_DENIED'):
                m.run_broadcast(**self.args)
            gate.assert_called_once()
            cast.assert_not_called()
            secrets.assert_not_called()

    def test_missing_hash_approval_fails_before_remote_checks(self):
        with patch.object(m,'checked_admission') as admission, patch.object(m.subprocess,'run') as cast:
            with self.assertRaises(m.admission.AdmissionError):
                m.run_broadcast(**{**self.args,'approved_quote_sha':'bad'})
            admission.assert_not_called();cast.assert_not_called()

    def test_wrong_quote_approval_never_checks_gas_or_secret(self):
        q={'quote_sha256':'d'*64,'deposit_address':'0x'+'1'*40}
        with patch.object(m,'checked_admission',return_value=q),patch.object(m,'check_broadcast_gas') as gas,\
             patch.object(m,'assert_credentials') as secrets,patch.object(m.subprocess,'run') as cast:
            with self.assertRaisesRegex(m.admission.AdmissionError,'USER_DID_NOT_APPROVE'):
                m.run_broadcast(**self.args)
            gas.assert_not_called();secrets.assert_not_called();cast.assert_not_called()

    def test_gas_cap_rejects_high_price(self):
        results=['0x14a34','0x186a0','0x77359400','0x1']
        with patch.object(m.admission,'rpc',side_effect=results):
            with self.assertRaisesRegex(m.admission.AdmissionError,'GAS_PRICE_EXCEEDS_CAP'):
                m.check_broadcast_gas('0xa9059cbb'+'0'*128)

    def test_dry_run_never_uses_keystore_or_broadcast(self):
        q={'quote_sha256':'c'*64,'deposit_address':'0x'+'1'*40}
        with patch.object(m,'checked_admission',return_value=q),\
             patch.object(m,'check_broadcast_gas',return_value=(1000000,2,65000)),\
             patch.object(m,'assert_credentials') as secrets,patch.object(m.subprocess,'run') as cast:
            x=m.run_broadcast(**{**self.args,'execute':False})
            self.assertFalse(x['broadcast']);self.assertEqual(x['classification'],'GUARD_ADMITTED_TESTNET_ONLY')
            secrets.assert_not_called();cast.assert_not_called()

    def test_presign_reverification_denial_prevents_cast(self):
        q={'quote_sha256':'c'*64,'deposit_address':'0x'+'1'*40}
        with patch.object(m,'checked_admission',side_effect=[q,m.admission.AdmissionError('GUARD_REVOKED')]) as gate, \
             patch.object(m,'check_broadcast_gas',return_value=(1000000,2,65000)), \
             patch.object(m,'assert_credentials'),patch.object(m.subprocess,'run') as cast:
            with self.assertRaisesRegex(m.admission.AdmissionError,'GUARD_REVOKED'):
                m.run_broadcast(**self.args)
            self.assertEqual(gate.call_count,2);cast.assert_not_called()

    def test_timeout_marked_indeterminate_and_replay_protected(self):
        q={'quote_sha256':'c'*64,'deposit_address':'0x'+'1'*40}
        with patch.object(m,'checked_admission',return_value=q), \
             patch.object(m,'check_broadcast_gas',return_value=(1000000,2,65000)), \
             patch.object(m,'assert_credentials'), \
             patch.object(m.subprocess,'run',side_effect=subprocess.TimeoutExpired(cmd='cast send',timeout=140)) as cast:
            result=m.run_broadcast(**self.args)
            self.assertEqual(result['classification'],'BROADCAST_INDETERMINATE_CHECK_CHAIN_BEFORE_RETRY')
            self.assertEqual(result['broadcast'],'unknown')
            self.assertTrue((m.KEYSTORE_DIR/('submitted-'+'c'*64+'.json')).exists())
            self.assertEqual(cast.call_count,1)

    def test_replay_flag_prevents_cast(self):
        q={'quote_sha256':'c'*64,'deposit_address':'0x'+'1'*40}
        (m.KEYSTORE_DIR/('submitted-'+'c'*64+'.json')).write_text('{}')
        with patch.object(m,'checked_admission',return_value=q),\
             patch.object(m,'check_broadcast_gas',return_value=(1000000,2,65000)),\
             patch.object(m,'assert_credentials'),patch.object(m.subprocess,'run') as cast:
            with self.assertRaisesRegex(m.admission.AdmissionError,'REPLAY_TRANSACTION_ALREADY'):
                m.run_broadcast(**self.args)
            cast.assert_not_called()

if __name__=='__main__': unittest.main()
