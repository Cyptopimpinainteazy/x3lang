import importlib.util
from pathlib import Path
import tempfile
import unittest
ROOT=Path(__file__).resolve().parents[2]
p=Path(__file__).with_name('base_sepolia_x3_intent.py')
spec=importlib.util.spec_from_file_location('x3_testnet_intent',p)
m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m)

class TestTestnetSource(unittest.TestCase):
    def test_real_x3lang_source_parses_typechecks_and_lowers(self):
        x=m.compile_testnet_intent()
        self.assertEqual(x['origin_chain'],'basesepolia')
        self.assertEqual(x['destination_account'],'x3lang.testnet')
        self.assertEqual(len(x['program_sha256']),64)
        self.assertEqual(x['origin_amount'],1_000_000)

    def mutated(self,before,after):
        data=m.SOURCE.read_text()
        self.assertIn(before,data)
        with tempfile.TemporaryDirectory() as d:
            p=Path(d)/'bad.x3';p.write_text(data.replace(before,after))
            with self.assertRaises(m.ProgramRefused):m.compile_testnet_intent(p)

    def test_reject_changed_recipient(self):
        self.mutated('receiver x3lang.testnet','receiver attacker.testnet')

    def test_reject_changed_amount(self):
        self.mutated('amount 1000000','amount 2000000')

    def test_reject_mainnet_chain(self):
        self.mutated('BaseSepolia.USDC','Ethereum.USDC')

    def test_reject_changed_refund(self):
        self.mutated('refund BaseSepolia.USDC to 0x5aF0c1BF95B3A158f6Fc48e3DED35903b2fa0721','refund BaseSepolia.USDC to 0x0000000000000000000000000000000000000001')

    def test_mainnet_oneclick_adapter_does_not_map_basesepolia(self):
        import near_intents
        self.assertNotIn('basesepolia',near_intents.CHAIN_TO_BLOCKCHAIN)

if __name__=='__main__':unittest.main()
