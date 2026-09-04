from typing import Dict, Any, List
import binascii
import importlib.util
import os

try:
    from Crypto.Hash import keccak

    def _keccak_256(data: bytes) -> bytes:
        return keccak.new(digest_bits=256, data=data).digest()
except ImportError:
    try:
        from eth_hash.auto import keccak as _keccak_256
    except ImportError:
        def _keccak_256(data: bytes) -> bytes:
            raise RuntimeError(
                "no keccak implementation available; install pycryptodome or eth-hash"
            )


def _load_registry():
    path = os.path.join(os.path.dirname(__file__), 'registry.py')
    if not os.path.exists(path):
        path = os.path.join(os.path.dirname(__file__), '..', 'registry.py')
    spec = importlib.util.spec_from_file_location('registry', path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


registry = _load_registry()


def _keccak_selector(signature: str) -> bytes:
    return _keccak_256(signature.encode())[:4]


def _encode_uint256(value: int) -> bytes:
    return value.to_bytes(32, 'big')


def _encode_address(value: str) -> bytes:
    if value.startswith('0x'):
        value = value[2:]
    addr = bytes.fromhex(value.rjust(40, '0'))
    return addr.rjust(32, b'\x00')


def _encode_address_array(values: List[str]) -> bytes:
    out = _encode_uint256(len(values))
    for addr in values:
        out += _encode_address(addr)
    return out


def _encode_swap_exact_tokens_for_tokens(amount_in: int, amount_out_min: int, path: List[str], recipient: str, deadline: int) -> bytes:
    selector = _keccak_selector('swapExactTokensForTokens(uint256,uint256,address[],address,uint256)')
    head = b''
    head += _encode_uint256(amount_in)
    head += _encode_uint256(amount_out_min)
    head += _encode_uint256(0x80)
    head += _encode_address(recipient)
    head += _encode_uint256(deadline)

    dynamic = _encode_address_array(path)
    return selector + head + dynamic


def _encode_deposit_to_x3(token: str, amount: int, recipient: str) -> bytes:
    selector = _keccak_selector('depositToX3(address,uint256,bytes)')
    head = b''
    head += _encode_address(token)
    head += _encode_uint256(amount)
    head += _encode_uint256(0x60)
    dynamic = _encode_uint256(len(bytes.fromhex(recipient[2:]))) if recipient.startswith('0x') else b''
    if not dynamic:
        dynamic = _encode_uint256(len(recipient))
    dynamic += recipient.encode() if not recipient.startswith('0x') else bytes.fromhex(recipient[2:])
    return selector + head + dynamic


def _encode_release_from_x3(token: str, amount: int, recipient: str) -> bytes:
    selector = _keccak_selector('releaseFromX3(address,uint256,bytes)')
    head = b''
    head += _encode_address(token)
    head += _encode_uint256(amount)
    head += _encode_uint256(0x60)
    dynamic = _encode_uint256(len(bytes.fromhex(recipient[2:]))) if recipient.startswith('0x') else b''
    if not dynamic:
        dynamic = _encode_uint256(len(recipient))
    dynamic += recipient.encode() if not recipient.startswith('0x') else bytes.fromhex(recipient[2:])
    return selector + head + dynamic


def emit(plan: Dict[str, Any], step: Dict[str, Any]) -> Dict[str, Any]:
    action = step.get('action', 'swap')
    amount = int(float(step.get('amount') or 0)) if step.get('amount') else 0
    gateway = registry.CONTRACT_ADDRESSES.get('x3_external_gateway', '0xX3ExternalGateway00000000000000000000000000')

    if action == 'lock':
        token = step.get('token') or step.get('from', '')
        recipient = step.get('recipient') or getattr(registry, 'DEFAULT_RECIPIENT', '0x0000000000000000000000000000000000000000')
        calldata_bytes = _encode_deposit_to_x3(token, amount, recipient)
        calldata = '0x' + binascii.hexlify(calldata_bytes).decode()
        return {
            'type': 'lock',
            'chain': 'ethereum',
            'token': token,
            'amount': amount,
            'calldata': calldata,
            'target_contract': gateway,
        }

    if action == 'release':
        token = step.get('token') or step.get('to', '')
        recipient = step.get('recipient') or getattr(registry, 'DEFAULT_RECIPIENT', '0x0000000000000000000000000000000000000000')
        calldata_bytes = _encode_release_from_x3(token, amount, recipient)
        calldata = '0x' + binascii.hexlify(calldata_bytes).decode()
        return {
            'type': 'release',
            'chain': 'ethereum',
            'token': token,
            'amount': amount,
            'calldata': calldata,
            'target_contract': gateway,
        }

    to_token = step.get('to')
    from_token = step.get('from')
    router = registry.CONTRACT_ADDRESSES.get('uniswap_v2_router', registry.CONTRACT_ADDRESSES.get('uniswap', '0xUniswapRouter000000000000000000000000'))
    recipient = getattr(registry, 'DEFAULT_RECIPIENT', '0x0000000000000000000000000000000000000000')

    path_tokens = [from_token, to_token]
    path_addresses = []
    for token in path_tokens:
        address = registry.TOKEN_ADDRESSES.get('ethereum', {}).get(token)
        if address is None:
            address = recipient
        path_addresses.append(address)

    calldata_bytes = _encode_swap_exact_tokens_for_tokens(amount, 0, path_addresses, recipient, 0)
    calldata = '0x' + binascii.hexlify(calldata_bytes).decode()

    return {
        'type': 'swap',
        'chain': 'ethereum',
        'dex': step.get('dex'),
        'from': from_token,
        'to': to_token,
        'amount': amount,
        'calldata': calldata,
        'target_contract': router
    }
