"""Fail-closed input and no-output tests; NO chain RPC calls."""
from __future__ import annotations
import json
import tempfile
import unittest
from pathlib import Path
from vault_origin_two_chain_cli import execute

class ReadonlyCommandSafetyTest(unittest.TestCase):
    def test_malformed_manifest_hard_denied_before_any_network(self):
        with tempfile.TemporaryDirectory() as tmp:
            p=Path(tmp)
            b=p/'base.json';n=p/'near.json';q=p/'quote.json';out=p/'out.json'
            b.write_text(json.dumps({'tx_hash':'0x'+'11'*32,'rpc_urls':['https://a.example']}))
            n.write_text('{}');q.write_text('{}')
            rc=execute(['--base-manifest',str(b),'--near-manifest',str(n),
                        '--signed-quote-file',str(q),'--output',str(out)])
            self.assertEqual(rc,2)
            self.assertFalse(out.exists())

    def test_unknown_quote_classification_does_not_emit_certificate(self):
        import dataclasses
        from vault_origin_base_proof import Expected
        from near_delivery_receipt_proof import NearExpected
        with tempfile.TemporaryDirectory() as tmp:
            p=Path(tmp)
            b=p/'base.json';n=p/'near.json';q=p/'quote.json';out=p/'out.json'
            e=Expected('0x'+'11'*20,'0x'+'22'*20,'0x'+'33'*20,'0x'+'44'*20,'0x'+'55'*20,
                       '1'*64,'2'*64,'3'*64,'4'*64,'0x'+'66'*32,1000,1800000100,1800000200,1000,1000)
            ner=NearExpected('A'*43,'signer.testnet','x3lang.testnet','usdc.tidepool-sandbox.testnet',900,
                             'B'*43,'C'*43,'x3guardlaunch2026.testnet','4'*64,'D'*43,'J'*43,
                             'x3lang.testnet','nonce-1','1'*64,'2'*64,1000,1800000000,800,
                             '0x'+'22'*20,100,1800000100)
            b.write_text(json.dumps({**dataclasses.asdict(e),'tx_hash':'0x'+'77'*32,
                 'rpc_urls':['https://a.example','https://b.example']}))
            n.write_text(json.dumps({**dataclasses.asdict(ner),
                 'rpc_urls':['https://na.example','https://nb.example']}))
            q.write_text(json.dumps({'classification':'SYNTHETIC_SIMULATED_SUCCESS','response':{}}))
            self.assertEqual(execute(['--base-manifest',str(b),'--near-manifest',str(n),
                '--signed-quote-file',str(q),'--output',str(out)]),2)
            self.assertFalse(out.exists())

if __name__=='__main__':unittest.main()
