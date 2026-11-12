"""Fail-closed input and no-output tests; NO chain RPC calls."""
from __future__ import annotations
from contextlib import redirect_stderr
from dataclasses import asdict
from datetime import timedelta
import io
import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch
import base_sepolia_guarded_admission as admission
from near_delivery_receipt_proof import NearExpected
from test_fresh_x3_proof_binding import NONCE, PROGRAM_SHA, SOURCE
from vault_origin_base_proof import Expected, ProofRejected
from vault_origin_two_chain_cli import REPO_ROOT, execute

FRESH_SOURCE_ARGS = ['--fresh-x3-source', str(SOURCE), '--expected-owner-nonce', NONCE,
                     '--expected-program-sha256', PROGRAM_SHA]

class ReadonlyCommandSafetyTest(unittest.TestCase):
    def test_malformed_manifest_hard_denied_before_any_network(self):
        with tempfile.TemporaryDirectory() as tmp:
            p=Path(tmp)
            b=p/'base.json';n=p/'near.json';q=p/'quote.json';out=p/'out.json'
            b.write_text(json.dumps({'tx_hash':'0x'+'11'*32,'rpc_urls':['https://a.example']}))
            n.write_text('{}');q.write_text('{}')
            self.assert_denied(b,n,q,out,'BASE_MANIFEST_FIELD_MISMATCH')

    def assert_denied(self,b,n,q,out,reason):
        stderr=io.StringIO()
        with redirect_stderr(stderr), patch('vault_origin_two_chain_cli._https_rpc') as base_rpc, patch('vault_origin_two_chain_cli._https_near_rpc') as near_rpc:
            rc=execute(['--base-manifest',str(b),'--near-manifest',str(n),
                        '--signed-quote-file',str(q),'--output',str(out),*FRESH_SOURCE_ARGS])
        self.assertEqual(rc,2)
        self.assertEqual(json.loads(stderr.getvalue())['reason'],reason)
        base_rpc.assert_not_called()
        near_rpc.assert_not_called()
        self.assertFalse(out.exists())

    def test_unknown_quote_classification_does_not_emit_certificate(self):
        with tempfile.TemporaryDirectory() as tmp:
            p=Path(tmp)
            b=p/'base.json';n=p/'near.json';q=p/'quote.json';out=p/'out.json'
            e=Expected('0x'+'11'*20,'0x'+'22'*20,'0x'+'33'*20,'0x'+'44'*20,'0x'+'55'*20,
                       '1'*64,'2'*64,'3'*64,'4'*64,'0x'+'66'*32,1000,1800000100,1800000200,1000,1000)
            ner=NearExpected('A'*43,'signer.testnet','x3lang.testnet','usdc.tidepool-sandbox.testnet',900,
                             'B'*43,'C'*43,'x3guardlaunch2026.testnet','4'*64,'D'*43,'J'*43,
                             'x3lang.testnet','nonce-1','1'*64,'2'*64,1000,1800000000,800,
                             '0x'+'22'*20,100,1800000100)
            b.write_text(json.dumps({**asdict(e),'tx_hash':'0x'+'77'*32,
                 'rpc_urls':['https://a.example','https://b.example']}))
            n.write_text(json.dumps({**asdict(ner),
                 'rpc_urls':['https://na.example','https://nb.example']}))
            q.write_text(json.dumps({'classification':'SYNTHETIC_SIMULATED_SUCCESS','response':{}}))
            self.assert_denied(b,n,q,out,'ORIGINAL_SIGNED_TIDEPOOL_QUOTE_REQUIRED')

    def test_signed_quote_failures_and_compiled_limits_are_checked_before_rpc(self):
        fixture=REPO_ROOT/'x3-lang/near/x3-tidepool-guard/src/signed_live_fixture.json'
        signed=json.loads(fixture.read_text())
        response=json.loads(signed['signed_payload'])
        response['signature']=signed['signature']
        quote=admission.verify_signed_quote(response)
        with tempfile.TemporaryDirectory() as tmp:
            p=Path(tmp)
            b=p/'base.json';n=p/'near.json';q=p/'quote.json';out=p/'out.json'
            deadline=int(quote['deadline'].timestamp())
            e=Expected('0x'+'11'*20,admission.SOURCE,'0x'+'33'*20,admission.TOKEN,
                       quote['deposit_address'],quote['quote_sha256'],PROGRAM_SHA,
                       admission.TOKEN_CODE_SHA256,'3'*64,'0x'+'66'*32,1_000_000,deadline,
                       deadline+1,1_000_000,1_000_000)
            ner=NearExpected('A'*43,'signer.testnet','x3lang.testnet','usdc.tidepool-sandbox.testnet',
                             quote['minimum_out'],'B'*43,'C'*43,'x3guardlaunch2026.testnet','4'*64,
                             'D'*43,'J'*43,'x3lang.testnet',NONCE,quote['quote_sha256'],PROGRAM_SHA,
                             1_000_000,deadline-1,980_000,admission.SOURCE,100,deadline)
            b.write_text(json.dumps({**asdict(e),'tx_hash':'0x'+'77'*32,
                 'rpc_urls':['https://a.example','https://b.example']}))
            n.write_text(json.dumps({**asdict(ner),'rpc_urls':['https://na.example','https://nb.example']}))
            def write_quote(payload):
                q.write_text(json.dumps({'classification':'LIVE_QUOTE_UNFUNDED_UNAUTHORIZED_NOT_SETTLED',
                                         'response':payload}))
            write_quote({**response,'quoteRequest':{**response['quoteRequest'],'refundTo':'0x'+'aa'*20}})
            self.assert_denied(b,n,q,out,'MAKER_SIGNATURE_INVALID')
            write_quote(response)
            for terms,reason in (
                ({'deadline':quote['issued_at']+timedelta(minutes=30,seconds=1)},'X3_COMPILED_TIMEOUT_EXCEEDED'),
                ({'minimum_out':979_999},'X3_COMPILED_MIN_RECEIVE_VIOLATED'),
            ):
                # Simulate authenticated terms to exercise the policy layer without signing.
                with self.subTest(reason=reason), patch.object(admission,'verify_signed_quote',
                                                               return_value={**quote,**terms}):
                    self.assert_denied(b,n,q,out,reason)
            # The fixture's signed lifetime is within the compiled limit. Stop at the Base boundary.
            with patch('near_guard_quorum.validate_guard_identity'), patch('vault_origin_two_chain_cli._https_rpc'), patch(
                'vault_origin_two_chain_cli.verify_base', side_effect=ProofRejected('TEST_STOP_AT_BASE')
            ) as base:
                stderr=io.StringIO()
                with redirect_stderr(stderr):
                    self.assertEqual(execute(['--base-manifest',str(b),'--near-manifest',str(n),
                        '--signed-quote-file',str(q),'--output',str(out),*FRESH_SOURCE_ARGS]),2)
                self.assertEqual(json.loads(stderr.getvalue())['reason'],'TEST_STOP_AT_BASE')
                base.assert_called_once()
                self.assertFalse(out.exists())

if __name__=='__main__':unittest.main()
