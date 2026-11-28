"""Execute compiler output on a local SVM with real System, SPL, ATA and Memo programs."""

import base64
from pathlib import Path
import unittest

from solders.hash import Hash
from solders.instruction import AccountMeta, Instruction
from solders.keypair import Keypair
from solders.litesvm import LiteSVM
from solders.pubkey import Pubkey
from solders.token import ID as TOKEN
from solders.token.associated import get_associated_token_address
from solders.token.state import Mint, TokenAccount
from solders.transaction import Transaction
from solders.transaction_metadata import FailedTransactionMetadata, TransactionMetadata

from coin import compile_coin
from coin.svm import ASSOCIATED_TOKEN_PROGRAM, MEMO_PROGRAM, MINT_SIZE, assemble_svm

SOURCE = (Path(__file__).resolve().parents[2] / "examples" / "dual_vm.x3").read_text()
# Public deterministic test identities for the isolated VM. Never fund these addresses.
GENESIS = str(Hash.from_bytes(bytes([51]) * 32))


class IssuanceTests(unittest.TestCase):
    def setUp(self):
        self.vm = LiteSVM()
        self.payer = Keypair()
        self.mint = Keypair.from_seed(bytes([17]) * 32)
        self.owner = Keypair.from_seed(bytes([34]) * 32)
        self.assertIsInstance(self.vm.airdrop(self.payer.pubkey(), 1_000_000_000), TransactionMetadata)

    def assemble(self, source=SOURCE, **overrides):
        kwargs = {
            "payer": str(self.payer.pubkey()),
            "recent_blockhash": str(self.vm.latest_blockhash()),
            "mint_rent_lamports": self.vm.minimum_balance_for_rent_exemption(MINT_SIZE),
            "observed_genesis_hash": GENESIS,
        }
        return assemble_svm(source, **(kwargs | overrides))

    def transaction(self, packet):
        tx = Transaction.from_bytes(base64.b64decode(packet["unsigned_transaction_base64"], validate=True))
        self.assertEqual(tx.verify_with_results(), [False, False])
        self.assertEqual(packet["required_signers"], [str(self.payer.pubkey()), str(self.mint.pubkey())])
        tx.sign([self.payer, self.mint], self.vm.latest_blockhash())
        return tx

    def test_atomic_issuance_and_fixed_supply(self):
        packet = self.assemble()
        tx = self.transaction(packet)
        result = self.vm.send_transaction(tx)
        self.assertIsInstance(result, TransactionMetadata, result)
        memo = "x3-coin-v1:" + compile_coin(SOURCE)["declaration_hash"]
        self.assertIn(memo, "\n".join(result.logs()))
        self.assertEqual(len(tx.message.instructions), 6)
        self.assertEqual(tx.message.account_keys[tx.message.instructions[-1].program_id_index],
                         Pubkey.from_string(MEMO_PROGRAM))

        mint_address = self.mint.pubkey()
        owner = self.owner.pubkey()
        ata = get_associated_token_address(owner, mint_address, TOKEN)
        self.assertEqual(str(ata), packet["recipient_token_account"])
        mint_account = self.vm.get_account(mint_address)
        holder_account = self.vm.get_account(ata)
        self.assertEqual(mint_account.owner, TOKEN)
        self.assertEqual(holder_account.owner, TOKEN)
        mint_state = Mint.from_bytes(bytes(mint_account.data))
        holder = TokenAccount.from_bytes(bytes(holder_account.data))
        self.assertEqual((mint_state.supply, mint_state.decimals, mint_state.is_initialized),
                         (400_000_000, 6, True))
        self.assertIsNone(mint_state.mint_authority)
        self.assertIsNone(mint_state.freeze_authority)
        self.assertEqual((holder.owner, holder.mint, holder.amount), (owner, mint_address, mint_state.supply))

        # The original mint key still signs, but the classic SPL program has
        # permanently removed its authority. Reusing a new blockhash also fails.
        extra_mint = Instruction(TOKEN, b"\x0e" + (1).to_bytes(8, "little") + b"\x06", [
            AccountMeta(mint_address, False, True), AccountMeta(ata, False, True),
            AccountMeta(mint_address, True, False),
        ])
        unauthorized = Transaction.new_signed_with_payer(
            [extra_mint], self.payer.pubkey(), [self.payer, self.mint], self.vm.latest_blockhash())
        denial = self.vm.send_transaction(unauthorized)
        self.assertIsInstance(denial, FailedTransactionMetadata)
        self.assertIn("the total supply of this token is fixed", "\n".join(denial.meta().logs()).lower())
        self.vm.expire_blockhash()
        replay = self.vm.send_transaction(self.transaction(self.assemble()))
        self.assertIsInstance(replay, FailedTransactionMetadata)
        self.assertEqual(Mint.from_bytes(bytes(self.vm.get_account(mint_address).data)).supply, 400_000_000)

        # Ordinary holders can still make a checked SPL transfer.
        receiver = Keypair().pubkey()
        dest_ata = get_associated_token_address(receiver, mint_address, TOKEN)
        link_account = Instruction(Pubkey.from_string(ASSOCIATED_TOKEN_PROGRAM), b"\x01", [
            AccountMeta(self.payer.pubkey(), True, True), AccountMeta(dest_ata, False, True),
            AccountMeta(receiver, False, False), AccountMeta(mint_address, False, False),
            AccountMeta(Pubkey.default(), False, False), AccountMeta(TOKEN, False, False),
        ])
        transfer = Instruction(TOKEN, b"\x0c" + (7).to_bytes(8, "little") + b"\x06", [
            AccountMeta(ata, False, True), AccountMeta(mint_address, False, False),
            AccountMeta(dest_ata, False, True), AccountMeta(owner, True, False),
        ])
        tx = Transaction.new_signed_with_payer(
            [link_account, transfer], self.payer.pubkey(), [self.payer, self.owner], self.vm.latest_blockhash())
        result = self.vm.send_transaction(tx)
        self.assertIsInstance(result, TransactionMetadata, result)
        self.assertEqual(TokenAccount.from_bytes(bytes(self.vm.get_account(dest_ata).data)).amount, 7)
        self.assertEqual(TokenAccount.from_bytes(bytes(self.vm.get_account(ata).data)).amount, 399_999_993)

    def test_big_integer_supply_and_mint_signature(self):
        # Above both JS Number's safe range and i64; none of the wire fields can round.
        amount = 2**64 - 3
        source = SOURCE.replace("cap 1000000000", f"cap {2**64 - 1}").replace(
            "allocation 600000000", "allocation 2").replace("allocation 400000000", f"allocation {amount}")
        packet = self.assemble(source)
        partial = Transaction.from_bytes(base64.b64decode(packet["unsigned_transaction_base64"]))
        partial.partial_sign([self.payer], self.vm.latest_blockhash())
        self.assertEqual(partial.verify_with_results(), [True, False])
        self.assertIsInstance(self.vm.send_transaction(partial), FailedTransactionMetadata)
        self.assertIsNone(self.vm.get_account(self.mint.pubkey()))
        result = self.vm.send_transaction(self.transaction(packet))
        self.assertIsInstance(result, TransactionMetadata, result)
        self.assertEqual(Mint.from_bytes(bytes(self.vm.get_account(self.mint.pubkey()).data)).supply, amount)

    def test_later_instruction_failure_reverts_creation(self):
        packet = self.assemble(mint_rent_lamports=1)
        result = self.vm.send_transaction(self.transaction(packet))
        self.assertIsInstance(result, FailedTransactionMetadata)
        self.assertIn("Instruction: InitializeMint2", "\n".join(result.meta().logs()))
        self.assertIsNone(self.vm.get_account(self.mint.pubkey()))
        self.assertIsNone(self.vm.get_account(Pubkey.from_string(packet["recipient_token_account"])))

    def test_reject_wrong_cluster_and_invalid_assembly_values(self):
        for overrides in (
            {"observed_genesis_hash": str(Hash.new_unique())},
            {"observed_genesis_hash": GENESIS[:32]},
            {"payer": str(self.mint.pubkey())},
            {"recent_blockhash": str(Hash.default())},
            {"mint_rent_lamports": 1.5},
            {"mint_rent_lamports": True},
            {"mint_rent_lamports": -1},
        ):
            with self.subTest(overrides=overrides):
                with self.assertRaises(ValueError):
                    self.assemble(**overrides)


if __name__ == "__main__":
    unittest.main()
