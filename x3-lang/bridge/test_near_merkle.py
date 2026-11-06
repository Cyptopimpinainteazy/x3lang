"""Protocol-level hashing tests. Synthetic vectors are NOT chain evidence."""
import hashlib
import unittest

from near_merkle import decode_hash, encode_hash, compute_merkle_root, lite_block_hash, verify_block_inclusion

H = lambda x: hashlib.sha256(x).digest()


class MerkleTests(unittest.TestCase):
    def test_base58_round_trip_32_bytes(self):
        for value in (b'\x00'*32, b'\xff'*32, H(b'near-hash')):
            self.assertEqual(decode_hash(encode_hash(value)),value)
        for value in ('0'*44, '111', 'A'*60, 123):
            with self.assertRaises(ValueError): decode_hash(value)

    def test_empty_path_is_identity(self):
        leaf = H(b'actual-leaf')
        self.assertEqual(compute_merkle_root(leaf, []), leaf)
        self.assertTrue(verify_block_inclusion(leaf, [], leaf))

    def test_left_right_merkle_concatenation(self):
        leaf = H(b'leaf')
        sib1 = H(b'left')
        sib2 = H(b'right')
        path=[{'hash':encode_hash(sib1),'direction':'Left'},
              {'hash':encode_hash(sib2),'direction':'Right'}]
        expected=H(H(sib1+leaf)+sib2)
        self.assertEqual(compute_merkle_root(leaf,path),expected)
        self.assertTrue(verify_block_inclusion(leaf,path,expected))
        self.assertFalse(verify_block_inclusion(leaf,path,H(b'wrong-root')))
        self.assertFalse(verify_block_inclusion(H(b'wrong-leaf'),path,expected))

    def test_changed_direction_rejected_by_comparison(self):
        leaf=H(b'leaf'); sib=H(b'sib')
        root=H(leaf+sib)
        self.assertTrue(verify_block_inclusion(leaf,[{'hash':encode_hash(sib),'direction':'Right'}],root))
        self.assertFalse(verify_block_inclusion(leaf,[{'hash':encode_hash(sib),'direction':'Left'}],root))

    def test_header_hash_matches_spec_nested_sha256_borsh(self):
        fields={
            'height':1234,'epoch_id':encode_hash(bytes([1])*32),
            'next_epoch_id':encode_hash(bytes([2])*32),
            'prev_state_root':encode_hash(bytes([3])*32),
            'outcome_root':encode_hash(bytes([4])*32),
            'timestamp':1700000000000000000,
            'next_bp_hash':encode_hash(bytes([5])*32),
            'block_merkle_root':encode_hash(bytes([6])*32)}
        header={'prev_block_hash':encode_hash(bytes([7])*32),
                'inner_rest_hash':encode_hash(bytes([8])*32), 'inner_lite':fields}
        # Independent reference construction: Borsh little-endian u64 + 32-byte hashes.
        borsh=(1234).to_bytes(8,'little')+bytes([1])*32+bytes([2])*32+bytes([3])*32+bytes([4])*32+\
              (1700000000000000000).to_bytes(8,'little')+bytes([5])*32+bytes([6])*32
        expected=H(H(H(borsh)+bytes([8])*32)+bytes([7])*32)
        self.assertEqual(lite_block_hash(header),expected)
        bad=dict(header); bad['inner_lite']=dict(fields,timestamp=1700000000000000001)
        self.assertNotEqual(lite_block_hash(bad),expected)

    def test_header_negative_malformed_fields(self):
        header={'prev_block_hash':encode_hash(bytes([7])*32),'inner_rest_hash':encode_hash(bytes([8])*32),
                'inner_lite':{'height':-1}}
        with self.assertRaises(ValueError): lite_block_hash(header)
        with self.assertRaises(ValueError): compute_merkle_root(H(b'x'), [{'hash':encode_hash(H(b'y')),'direction':'Up'}])

if __name__ == '__main__': unittest.main()

class ReceiptBlockProofTests(unittest.TestCase):
    def _proof(self):
        from near_receipt_proof import get_candidate
        zero=encode_hash(b'\x00'*32)
        inner={k:zero for k in ('epoch_id','next_epoch_id','prev_state_root','outcome_root','next_bp_hash','block_merkle_root')}
        inner.update(height=100,timestamp=1700000000000000000)
        header={'prev_block_hash':zero,'inner_rest_hash':zero,'inner_lite':inner}
        block=encode_hash(lite_block_hash(header))
        proof={'jsonrpc':'2.0','id':'x3-near-light-proof','result':{
            'outcome_proof':{'id':zero,'block_hash':block,'proof':[],
                'outcome':{'executor_id':'escrow.testnet','status':{'SuccessValue':''},'logs':[]}},
            'outcome_root_proof':[], 'block_header_lite':header,'block_proof':[]}}
        return proof, block, zero

    def test_block_header_hash_and_merkle_membership_agree(self):
        from near_merkle import check_receipt_block_merkle
        proof, block, rid=self._proof()
        check=check_receipt_block_merkle(proof, receipt_id=rid,
                   receiver_id='escrow.testnet', light_client_head=rid,
                   expected_head_block_merkle_root=decode_hash(block))
        self.assertTrue(check.merkle_path_matches)
        self.assertFalse(check.authenticated)
        self.assertFalse(check.authorized_to_sign)

    def test_wrong_head_merkle_root_is_rejected(self):
        from near_merkle import check_receipt_block_merkle
        proof, _, rid=self._proof()
        with self.assertRaises(ValueError):
            check_receipt_block_merkle(proof,receipt_id=rid,
                receiver_id='escrow.testnet',light_client_head=rid,
                expected_head_block_merkle_root=H(b'fake-checkpoint'))

    def test_outcome_header_block_hash_mismatch_is_rejected(self):
        from near_merkle import check_receipt_block_merkle
        proof, block, rid=self._proof()
        proof['result']['outcome_proof']['block_hash']=rid
        with self.assertRaises(ValueError):
            check_receipt_block_merkle(proof,receipt_id=rid,
                receiver_id='escrow.testnet',light_client_head=rid,
                expected_head_block_merkle_root=decode_hash(block))

    def test_block_proof_wrong_direction_is_rejected(self):
        from near_merkle import check_receipt_block_merkle
        proof, block, rid=self._proof()
        proof['result']['block_proof']=[{'hash':rid,'direction':'Right'}]
        with self.assertRaises(ValueError):
            check_receipt_block_merkle(proof,receipt_id=rid,
                receiver_id='escrow.testnet',light_client_head=rid,
                expected_head_block_merkle_root=decode_hash(block))
