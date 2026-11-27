"""Generate fixed-supply classic SPL issuance. This module only builds unsigned data."""

import base64

TOKEN_PROGRAM = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA"
ASSOCIATED_TOKEN_PROGRAM = "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL"
MEMO_PROGRAM = "MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr"
MINT_SIZE = 82
MAX_U64 = 2**64 - 1


def build_svm(coin):
    """Describe one unsigned, atomic allocation from validated compiler input."""
    target = coin["svm"]
    return {
        "format": "x3-coin-spl-v1",
        "genesis_hash": target["genesis_hash"],
        "mint": target["mint"],
        "recipient": target["recipient"],
        "allocation": str(target["allocation"]),
        "decimals": coin["decimals"],
        "program": TOKEN_PROGRAM,
        "memo": "x3-coin-v1:" + coin["declaration_hash"],
        "operations": [
            "CreateAccount", "InitializeMint2", "CreateIdempotent",
            "MintToChecked", "SetAuthority(MintTokens,None)", "Memo",
        ],
    }


def assemble_svm(source, *, observed_genesis_hash, payer, recent_blockhash, mint_rent_lamports):
    """Build the complete unsigned wire transaction, recompiling the reviewed source.

    The caller supplies trusted getGenesisHash, getLatestBlockhash, and
    getMinimumBalanceForRentExemption(82) results from the SAME cluster.
    Genesis comparison is an off-chain guard. The recent blockhash binds the
    message to recent chain state. The fee payer also funds the recipient ATA.
    Only the payer and the new mint account sign the resulting transaction.
    """
    from solders.hash import Hash
    from solders.instruction import AccountMeta, Instruction
    from solders.message import Message
    from solders.pubkey import Pubkey
    from solders.system_program import ID as system_program, CreateAccountParams, create_account
    from solders.token.associated import get_associated_token_address
    from solders.transaction import Transaction

    from . import compile_coin

    plan = compile_coin(source)["svm_issuance"]
    if observed_genesis_hash != plan["genesis_hash"]:
        raise ValueError("observed cluster genesis hash differs from the declared genesis hash")
    if type(mint_rent_lamports) is not int or not 0 < mint_rent_lamports <= MAX_U64:
        raise ValueError("mint_rent_lamports must be a positive u64")
    blockhash = Hash.from_string(recent_blockhash)
    if blockhash == Hash.default():
        raise ValueError("a recent nonzero blockhash is required")

    fee_payer = Pubkey.from_string(payer)
    mint = Pubkey.from_string(plan["mint"])
    owner = Pubkey.from_string(plan["recipient"])
    if fee_payer in (mint, system_program) or not fee_payer.is_on_curve():
        raise ValueError("fee payer must be a signer distinct from the new mint")
    token = Pubkey.from_string(TOKEN_PROGRAM)
    ata_program = Pubkey.from_string(ASSOCIATED_TOKEN_PROGRAM)
    ata = get_associated_token_address(owner, mint, token)
    decimals = plan["decimals"]
    amount = int(plan["allocation"])
    mint_writable = AccountMeta(mint, False, True)
    mint_authority = AccountMeta(mint, True, False)
    ata_writable = AccountMeta(ata, False, True)

    # SPL Token's instruction COption encoding is a single tag byte for None.
    # InitializeMint2 sets the mint account itself as the temporary signer.
    instructions = [
        create_account(CreateAccountParams(
            from_pubkey=fee_payer, to_pubkey=mint, lamports=mint_rent_lamports,
            space=MINT_SIZE, owner=token,
        )),
        Instruction(token, bytes([20, decimals]) + bytes(mint) + b"\x00", [mint_writable]),
        Instruction(ata_program, b"\x01", [
            AccountMeta(fee_payer, True, True), ata_writable, AccountMeta(owner, False, False),
            AccountMeta(mint, False, False), AccountMeta(system_program, False, False),
            AccountMeta(token, False, False),
        ]),
        Instruction(token, b"\x0e" + amount.to_bytes(8, "little") + bytes([decimals]),
                    [mint_writable, ata_writable, mint_authority]),
        Instruction(token, b"\x06\x00\x00", [mint_writable, mint_authority]),
        Instruction(Pubkey.from_string(MEMO_PROGRAM), plan["memo"].encode("ascii"), []),
    ]
    message = Message.new_with_blockhash(instructions, fee_payer, blockhash)
    unsigned = Transaction.new_unsigned(message)
    return {
        "unsigned_transaction_base64": base64.b64encode(bytes(unsigned)).decode("ascii"),
        "recipient_token_account": str(ata),
        "required_signers": [str(key) for key in message.account_keys[:message.header.num_required_signatures]],
        "mint_rent_lamports": str(mint_rent_lamports),
        "genesis_hash": observed_genesis_hash,
        "memo": plan["memo"],
    }
