import json
import unittest

from abi_import import inspect_abi, AbiError


def fn(name, types, mutability="nonpayable"):
    return dict(type="function", name=name, stateMutability=mutability,
                inputs=[{"type": t} for t in types], outputs=[])


def inspect(*items):
    return inspect_abi(json.dumps(items).encode())


class AbiTests(unittest.TestCase):
    def test_transfer_and_overload(self):
        report = inspect(fn("transfer", ["address", "uint256"]),
                         fn("transfer", ["address"]), fn("balanceOf", ["address"], "view"))
        rows = {r["signature"]: r for r in report["functions"]}
        self.assertEqual(rows["transfer(address,uint256)"]["selector"], "0xa9059cbb")
        self.assertNotEqual(rows["transfer(address)"]["selector"], "0xa9059cbb")
        self.assertTrue(rows["transfer(address,uint256)"]["profile_candidate"])
        self.assertFalse(rows["balanceOf(address)"]["profile_candidate"])
        self.assertFalse(report["authority_verified"])

    def test_unsupported_and_nonfunctions(self):
        report = inspect(fn("execute", ["bytes"]), fn("batch", ["uint256[]"]),
                         fn("wide", ["uint256"] * 17), {"type": "receive"})
        self.assertEqual(len(report["functions"]), 3)
        self.assertTrue(all(not r["profile_candidate"] for r in report["functions"]))
        self.assertEqual(report["nonfunctions"], ["receive"])
        self.assertTrue(report["incomplete_inventory"])

    def test_collision(self):
        report = inspect(fn("burn", ["uint256"]), fn("collate_propagate_storage", ["bytes16"]))
        self.assertEqual({r["selector"] for r in report["functions"]}, {"0x42966c68"})
        self.assertTrue(all(not r["profile_candidate"] for r in report["functions"]))

    def test_malformed(self):
        for raw in (b'{}', b'[{"type":"function","type":"event"}]',
                    json.dumps([fn("x", ["uint256"]), fn("x", ["uint256"])]).encode(),
                    json.dumps([fn("x", ["uint256"], "maybe")]).encode(),
                    b'[{"type":"function","name":"x","inputs":[]}]'):
            with self.assertRaises(AbiError):
                inspect_abi(raw)
