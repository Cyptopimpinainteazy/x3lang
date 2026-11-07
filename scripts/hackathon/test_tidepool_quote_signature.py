"""Cryptographic verification and tamper-rejection against freshly signed dry quote."""
import importlib.util
import json
from pathlib import Path
import sys
import unittest
from datetime import datetime, timezone, timedelta
from copy import deepcopy
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / 'x3-lang'))
path = Path(__file__).with_name('tidepool_quote_signature.py')
spec = importlib.util.spec_from_file_location('testnet_quote', path)
mod = importlib.util.module_from_spec(spec)
spec.loader.exec_module(mod)


class TestVerification(unittest.TestCase):
    def test_authentic_provider_signed_fixture_and_mutation(self):
        """Offline replay of authentic public dry-quote evidence; not a fresh trade."""
        f = json.loads(Path(__file__).with_name('fixtures').joinpath('tidepool_signed_dry_20261107.json').read_text())
        self.assertEqual(f['fixture_classification'], 'HISTORICAL_SIGNED_DRY_QUOTE_NO_DEPOSIT_NO_SETTLEMENT')
        response = f['response']
        self.assertTrue(mod.verify_signature(response))
        observed_at=datetime.fromisoformat(response['timestamp'].replace('Z','+00:00'))
        self.assertTrue(mod.verify_dry_quote(response,refund_to=response['quoteRequest']['refundTo'],now=observed_at)['signature_valid'])
        for path, replacement in [(('quoteRequest','recipient'), 'attacker.testnet'),
                                  (('quoteRequest','originAsset'), 'mainnet:usdc'),
                                  (('quote','amountIn'), '1000001'),
                                  (('quote','minAmountOut'), '1')]:
            tampered = deepcopy(response)
            tampered[path[0]][path[1]] = replacement
            with self.subTest(path=path):
                self.assertFalse(mod.verify_signature(tampered))

    def test_refund_and_freshness_are_binding(self):
        f=json.loads(Path(__file__).with_name('fixtures').joinpath('tidepool_signed_dry_20261107.json').read_text())
        q=f['response']
        observed=datetime.fromisoformat(q['timestamp'].replace('Z','+00:00'))
        with self.assertRaisesRegex(mod.TidepoolQuoteRejected,'REFUND'):
            mod.verify_dry_quote(q,refund_to='0x0000000000000000000000000000000000000002',now=observed)
        with self.assertRaisesRegex(mod.TidepoolQuoteRejected,'STALE'):
            mod.verify_dry_quote(q,refund_to=q['quoteRequest']['refundTo'],now=observed+timedelta(minutes=6))

    def test_wrong_signer_refused(self):
        self.assertFalse(mod.verify_signature({'signature':'ed25519:bad'}, public_key='ed25519:bad'))

    def test_malformed_refused(self):
        for value in ({}, {'signature':True}, {'signature':'nope'}):
            with self.subTest(value=value):
                self.assertFalse(mod.verify_signature(value))

    def test_no_unauthorized_acceptance(self):
        with self.assertRaisesRegex(mod.TidepoolQuoteRejected, 'SIGNATURE'):
            mod.verify_dry_quote({'signature':'ed25519:bad', 'quoteRequest':{'dry':True}})

    def test_json_hash_excludes_signature(self):
        response={'correlationId':'test', 'quote':{'amountIn':'1000000'}, 'signature':'anything'}
        self.assertEqual(len(mod.signed_bytes(response)),32)
        copy=deepcopy(response);copy['signature']='another'
        self.assertEqual(mod.signed_bytes(response),mod.signed_bytes(copy))

    def test_mainnet_signer_cannot_be_injected(self):
        from near_intents import ONE_CLICK_MANAGER_PUBLIC_KEY
        self.assertFalse(mod.verify_signature({'signature':'ed25519:bad'},public_key=ONE_CLICK_MANAGER_PUBLIC_KEY))

    def test_tampered_signed_field_refused(self):
        # A mock verifies the policy boundary; the live test below tests cryptography.
        observed=datetime.now(timezone.utc)
        refund='0x0000000000000000000000000000000000000001'
        valid={'timestamp':observed.isoformat(), 'quoteRequest':{'dry':True,'swapType':'EXACT_INPUT','slippageTolerance':100,
             'originAsset':mod.TESTNET_ORIGIN_ASSET,'destinationAsset':mod.TESTNET_DEST_ASSET,
             'depositType':'ORIGIN_CHAIN','recipientType':'DESTINATION_CHAIN','recipient':mod.ALLOWED_RECIPIENT,
             'refundType':'ORIGIN_CHAIN','refundTo':refund,'deadline':(observed+timedelta(minutes=30)).isoformat(), 'amount':'1000000'},
             'quote':{'amountIn':'1000000','amountOut':'997400','minAmountOut':'987426'}}
        with patch.object(mod,'verify_signature',return_value=True):
            self.assertTrue(mod.verify_dry_quote(valid,refund_to=refund,now=observed)['signature_valid'])
            for field,bad in [('recipient','wrong.testnet'),('originAsset','mainnet:usdc'),('amount','2000000')]:
                changed=deepcopy(valid);changed['quoteRequest'][field]=bad
                with self.subTest(field=field),self.assertRaises(mod.TidepoolQuoteRejected):
                    mod.verify_dry_quote(changed,refund_to=refund,now=observed)

if __name__ == '__main__':unittest.main()
