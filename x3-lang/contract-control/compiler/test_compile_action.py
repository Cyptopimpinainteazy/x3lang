import unittest
from compile_action import compile_action, PolicyError

SRC = b'''contract_policy treasury {
 chain_id 84532
 executor 0x1111111111111111111111111111111111111111
 target 0x2222222222222222222222222222222222222222
 agent 0x3333333333333333333333333333333333333333
 calldata 0x12345678
 max_value 10
 expiry 2000
}'''

def compile_it(source=SRC, **kw):
    args = dict(value=4, nonce=9, policy_version=2, now=1000)
    args.update(kw)
    return compile_action(source, **args)

class Tests(unittest.TestCase):
    def test_exact_action(self):
        m=compile_it()
        self.assertEqual(m['action'].get('selector'),'0x12345678')
        self.assertEqual(m['action']['value'],'4')
        self.assertFalse(m['owner_authorized'])
        self.assertEqual(m,compile_it())
        self.assertEqual(m['action_id'], '0xf1378e67992f207c9569ea986c2c9e003acc511b061be03ecd7ceffde9d8de8e')
    def test_mutations(self):
        initial=compile_it()['action_id']
        for change in ({'value':5},{'nonce':10},{'policy_version':3}):
            self.assertNotEqual(initial,compile_it(**change)['action_id'])
        for old,new in [(b'1111',b'4444'),(b'2222',b'5555'),(b'3333',b'6666'),(b'12345678',b'87654321'),(b'2000',b'2001'),(b'treasury',b'other')]:
            self.assertNotEqual(initial,compile_it(SRC.replace(old,new))['action_id'])
    def test_fail_closed(self):
        for changes in ({'value':11},{'now':2000},{'nonce':-1},{'policy_version':0},{'value':True},{'nonce':2**256}):
            with self.assertRaises(PolicyError): compile_it(**changes)
        for source in [SRC.replace(b'84532',b'1'), SRC.replace(b' max_value 10',b' max_value 10\n max_value 100'), SRC.replace(b' max_value 10',b' allow everything'), SRC.replace(b'0x12345678',b'0x1234567890'), SRC+b'\nallow anything', SRC.replace(b'0x1111111111111111111111111111111111111111',b'0x2222222222222222222222222222222222222222')]:
            with self.assertRaises(PolicyError): compile_it(source)
