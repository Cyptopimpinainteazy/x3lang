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
        # Last-resort pure-Python Keccak (Ethereum domain padding). Audited
        # backends (pycryptodome / eth-hash) are always preferred; this keeps
        # calldata encoding working in bare environments. The fallback is
        # self-verified against the canonical vectors on import; it never
        # silently substitutes NIST SHA3, which would corrupt EVM selectors.
        import importlib.util as _ilu
        _path = os.path.join(os.path.dirname(__file__), '_keccak.py')
        _spec = _ilu.spec_from_file_location('_keccak', _path)
        _mod = _ilu.module_from_spec(_spec)
        _spec.loader.exec_module(_mod)
        # Fail-closed: refuse to latch a broken digest provider onto a
        # security-sensitive encode path. The fallback must reproduce the two
        # canonical Keccak-256 vectors exactly.
        _expect = (
            'c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470',
            '4e03657aea45a94fc7d47ba826c8d667c0d1e6e33a64a036ec44f58fa12d6c45',
        )
        _got = (_mod.keccak_256(b'').hex(), _mod.keccak_256(b'abc').hex())
        if _got != _expect:
            raise RuntimeError('keccak-256 fallback failed self-verification')
        _keccak_256 = _mod.keccak_256


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
