from typing import Dict, Any, List
import importlib.util
import os


def _load_registry():
    path = os.path.join(os.path.dirname(__file__), '..', 'registry.py')
    spec = importlib.util.spec_from_file_location('registry', path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


registry = _load_registry()

BASE58_ALPHABET = '123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz'


def _base58_encode(data: bytes) -> str:
    num = int.from_bytes(data, 'big')
    if num == 0:
        return '1'
    encoded = ''
    while num > 0:
        num, rem = divmod(num, 58)
        encoded = BASE58_ALPHABET[rem] + encoded
    n_pad = len(data) - len(data.lstrip(b'\x00'))
    return '1' * n_pad + encoded


def _base58_decode(value: str) -> bytes:
    num = 0
    for char in value:
        num = num * 58 + BASE58_ALPHABET.index(char)
    decoded = num.to_bytes((num.bit_length() + 7) // 8, 'big')
    n_pad = len(value) - len(value.lstrip('1'))
    return b'\x00' * n_pad + decoded


def _encode_u64(value: int) -> bytes:
    return value.to_bytes(8, 'little')


def _encode_pubkey(pubkey: str) -> bytes:
    if pubkey.startswith('0x'):
        raw = bytes.fromhex(pubkey[2:])
        return raw.rjust(32, b'\x00')
    return _base58_decode(pubkey).rjust(32, b'\x00')


def _serialize_swap_instruction(amount: int, min_amount_out: int, source_mint: str, dest_mint: str) -> bytes:
    instruction_tag = b'\x01'
    payload = instruction_tag
    payload += _encode_u64(amount)
    payload += _encode_u64(min_amount_out)
    payload += _encode_pubkey(source_mint)
    payload += _encode_pubkey(dest_mint)
    return payload


def _serialize_kernel_mint_instruction(mint: str, amount: int, destination: str) -> bytes:
    instruction_tag = b'\x02'
    payload = instruction_tag
    payload += _encode_pubkey(mint)
    payload += _encode_u64(amount)
    payload += _encode_pubkey(destination)
    return payload


def _serialize_kernel_burn_instruction(mint: str, amount: int, source: str) -> bytes:
    instruction_tag = b'\x03'
    payload = instruction_tag
    payload += _encode_pubkey(mint)
    payload += _encode_u64(amount)
    payload += _encode_pubkey(source)
    return payload


def emit(plan: Dict[str, Any], step: Dict[str, Any]) -> Dict[str, Any]:
    action = step.get('action', 'swap')

    if action in ('mint', 'burn'):
        mint = step.get('mint') or step.get('token') or registry.TOKEN_ADDRESSES.get('solana', {}).get(step.get('from', 'SOL'), registry.TOKEN_ADDRESSES['solana']['SOL'])
        amount = int(float(step.get('amount') or 0)) if step.get('amount') else 0
        party = step.get('destination' if action == 'mint' else 'source') or step.get('recipient') or getattr(registry, 'DEFAULT_SVM_RECIPIENT', '11111111111111111111111111111111')
        kernel_program = registry.CONTRACT_ADDRESSES.get('x3_kernel', 'X3Kernel1111111111111111111111111111111111')

        if action == 'mint':
            data_bytes = _serialize_kernel_mint_instruction(mint, amount, party)
        else:
            data_bytes = _serialize_kernel_burn_instruction(mint, amount, party)

        data_b58 = _base58_encode(data_bytes)
        raw_data_hex = '0x' + data_bytes.hex()

        accounts = [
            {'pubkey': mint, 'is_signer': False, 'is_writable': True},
            {'pubkey': party, 'is_signer': False, 'is_writable': True},
            {'pubkey': kernel_program, 'is_signer': False, 'is_writable': False},
            {'pubkey': 'TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA', 'is_signer': False, 'is_writable': False},
        ]

        return {
            'type': action,
            'chain': 'solana',
            'mint': mint,
            'amount': amount,
            'program': kernel_program,
            'accounts': accounts,
            'data': data_b58,
            'raw_data_hex': raw_data_hex,
            'note': f'SVM kernel {action} instruction',
        }

    from_token = step.get('from')
    to_token = step.get('to')
    amount = int(float(step.get('amount') or 0)) if step.get('amount') else 0
    program = registry.CONTRACT_ADDRESSES.get('raydium', '4k3Dyjzvzp8e2y8bKE8xUrg8rXQZkmh8Y2xS1QZsbJt4')

    source_mint = registry.TOKEN_ADDRESSES.get('solana', {}).get(from_token, registry.TOKEN_ADDRESSES['solana']['SOL'])
    dest_mint = registry.TOKEN_ADDRESSES.get('solana', {}).get(to_token, registry.TOKEN_ADDRESSES['solana']['WSOL'])

    data_bytes = _serialize_swap_instruction(amount, 0, source_mint, dest_mint)
    data_b58 = _base58_encode(data_bytes)
    raw_data_hex = '0x' + data_bytes.hex()

    accounts = [
        {'pubkey': source_mint, 'is_signer': False, 'is_writable': True},
        {'pubkey': dest_mint, 'is_signer': False, 'is_writable': True},
        {'pubkey': program, 'is_signer': False, 'is_writable': False},
        {'pubkey': 'TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA', 'is_signer': False, 'is_writable': False}
    ]

    return {
        'type': 'swap',
        'chain': 'solana',
        'dex': step.get('dex'),
        'from': from_token,
        'to': to_token,
        'amount': step.get('amount'),
        'program': program,
        'accounts': accounts,
        'data': data_b58,
        'raw_data_hex': raw_data_hex,
        'note': 'SVM instruction payload'
    }
