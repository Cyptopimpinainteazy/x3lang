"""Production registries for chains, assets, DEXs, bridges and address validation."""
import base64
import binascii
import hashlib
import re

CHAIN_DOMAINS = {
    'solana': {'aliases': {'sol', 'svm'}, 'address': 'base58'},
    'ethereum': {'aliases': {'eth', 'evm'}, 'address': 'evm'},
    'arbitrum': {'aliases': {'arb'}, 'address': 'evm'},
    # Explicit *testnet-only* domain; deliberately not mapped in the production 1Click adapter.
    'basesepolia': {'aliases': {'base_sepolia'}, 'address': 'evm'},
    'polygon': {'aliases': {'matic'}, 'address': 'evm'},
    'bitcoin': {'aliases': {'btc'}, 'address': 'btc'},
    'bsc': {'aliases': {'bnb', 'binance', 'bnbchain'}, 'address': 'evm'},
    # EVM chains routed through NEAR Intents 1Click (`near_intents.CHAIN_TO_BLOCKCHAIN`), each
    # checked against 1Click's live `GET /v0/tokens` on 2026-11-16.
    'base': {'aliases': set(), 'address': 'evm'},
    'optimism': {'aliases': {'op', 'op_mainnet'}, 'address': 'evm'},
    'avalanche': {'aliases': {'avax', 'avalanche_c'}, 'address': 'evm'},
    'gnosis': {'aliases': {'xdai', 'gnosis_chain'}, 'address': 'evm'},
    'scroll': {'aliases': set(), 'address': 'evm'},
    'monad': {'aliases': set(), 'address': 'evm'},
    'berachain': {'aliases': {'bera'}, 'address': 'evm'},
    'abstract': {'aliases': {'abs'}, 'address': 'evm'},
    'plasma': {'aliases': {'xpl'}, 'address': 'evm'},
    'xlayer': {'aliases': {'x_layer', 'okx_layer'}, 'address': 'evm'},
    # Non-EVM chains routed through 1Click. Each has its own address check below, with its
    # real checksum: 1Click sends real funds to whatever address the program names.
    'tron': {'aliases': {'trx'}, 'address': 'tron'},
    'ton': {'aliases': set(), 'address': 'ton'},
    # UTXO chains routed through 1Click, each with its own address encoding and checksum.
    'dogecoin': {'aliases': {'doge'}, 'address': 'doge'},
    'litecoin': {'aliases': {'ltc'}, 'address': 'ltc'},
    'bitcoincash': {'aliases': {'bch', 'bitcoin_cash'}, 'address': 'bch'},
    'zcash': {'aliases': {'zec'}, 'address': 'zec'},
    'dash': {'aliases': set(), 'address': 'dash'},
    'xrp': {'aliases': {'ripple', 'xrpl'}, 'address': 'xrp'},
    'sui': {'aliases': set(), 'address': 'move32'},
    'aptos': {'aliases': {'apt'}, 'address': 'move32'},
    'movement': {'aliases': set(), 'address': 'move32'},
    'x3': {'aliases': set(), 'address': 'x3'},
    # NEAR is a destination and origin for `bridge NearIntents` legs, which settle through the
    # NEAR Intents 1Click API (`near_intents.py`). Its receivers are NEAR account IDs.
    'near': {'aliases': set(), 'address': 'near'},
}

ALIASES = {alias: chain for chain, meta in CHAIN_DOMAINS.items() for alias in meta['aliases']}
ALIASES.update({chain: chain for chain in CHAIN_DOMAINS})

ASSETS_BY_CHAIN = {
    'solana': {'USDC', 'SOL', 'WSOL', 'USDT', 'BONK'},
    'ethereum': {'USDC', 'USDT', 'WETH', 'ETH', 'WSOL', 'DAI'},
    'arbitrum': {'USDC', 'USDT', 'WETH', 'ETH', 'ARB'},
    'basesepolia': {'USDC', 'USDT'},
    'polygon': {'USDC', 'USDT', 'WETH', 'MATIC', 'DAI'},
    'bitcoin': {'BTC', 'WBTC'},
    'bsc': {'BNB', 'USDC', 'USDT'},
    # Symbols exactly as 1Click lists them on each chain (`GET /v0/tokens`, 2026-11-16).
    'base': {'ETH', 'WETH', 'USDC', 'cbBTC', 'BRETT', 'KAITO', 'VVV'},
    'optimism': {'ETH', 'WETH', 'USDC', 'USDT', 'USDT0', 'OP'},
    'avalanche': {'AVAX', 'USDC', 'USDT'},
    'gnosis': {'xDAI', 'USDC', 'USDT', 'WETH', 'GNO', 'COW', 'SAFE', 'EURe', 'GBPe'},
    'scroll': {'ETH', 'USDT'},
    'monad': {'MON', 'USDC', 'USDT0'},
    'berachain': {'BERA', 'USDT0'},
    'abstract': {'ETH'},
    'plasma': {'XPL', 'USDT0'},
    'xlayer': {'OKB', 'USDC', 'USDT0'},
    'tron': {'TRX', 'USDT'},
    # 1Click lists the native coin as GRAM; `TON` is accepted and resolved to it.
    'ton': {'TON', 'GRAM', 'USDT'},
    'dogecoin': {'DOGE'},
    'litecoin': {'LTC'},
    'bitcoincash': {'BCH'},
    'zcash': {'ZEC'},
    'dash': {'DASH'},
    'xrp': {'XRP'},
    'sui': {'SUI', 'USDC'},
    'aptos': {'APT', 'USDC', 'USDT'},
    'movement': {'MOVE', 'USDCx'},
    'x3': {'SOL', 'USDC', 'WSOL', 'WETH', 'BTC', 'X3'},
    'near': {'NEAR', 'wNEAR', 'USDC', 'USDT'},
}

# Chains people ask for that no route adapter here can settle. They are refused **by name**, so
# a program targeting one gets a reason instead of "unsupported chain", and nothing pretends
# to route it. Each was absent from 1Click's live `GET /v0/tokens` on 2026-11-16; 1Click's
# `hypercore` is Hyperliquid's core chain, not HyperEVM. Moving a chain out of here needs a
# real adapter (or 1Click listing it) and tests, in the same change.
NOT_ROUTABLE_CHAINS = {
    'mantle': {'aliases': {'mnt'}},
    'zksync': {'aliases': {'zksync_era', 'zksyncera'}},
    'linea': {'aliases': set()},
    'cronos': {'aliases': {'cro'}},
    'moonbeam': {'aliases': {'glmr'}},
    'celo': {'aliases': set()},
    'plume': {'aliases': set()},
    'aurora': {'aliases': set()},
    'hyperevm': {'aliases': {'hyperliquid_evm', 'hyperliquid'}},
}
NOT_ROUTABLE_ALIASES = {alias: chain for chain, meta in NOT_ROUTABLE_CHAINS.items() for alias in meta['aliases']}
NOT_ROUTABLE_ALIASES.update({chain: chain for chain in NOT_ROUTABLE_CHAINS})


def not_routable(chain):
    """The canonical name of a chain known to have no route adapter here, or None."""
    if chain is None:
        return None
    return NOT_ROUTABLE_ALIASES.get(str(chain).lower())


KNOWN_DEXS = {'raydium', 'orca', 'uniswap', 'sushiswap', 'curve'}
# The spellings of the NEAR Intents adapter. The parser lowercases `via`, so `NearIntents` in
# source arrives as `nearintents`; `near_intents` is accepted so either spelling works.
NEAR_INTENTS_VIAS = {'nearintents', 'near_intents'}
KNOWN_BRIDGES = {'x3', 'wormhole', 'layerzero', 'axelar'} | NEAR_INTENTS_VIAS
SUPPORTED_OPERATIONS = {'swap', 'bridge', 'lock', 'mint', 'burn', 'release'}
# The guard kinds this surface reads. It is a **subset** of the compiler's — the Rust
# `require_kind_from_str` knows eighteen, this knows nine — and that is a deliberate
# scope, not an accident to be closed. What is not deliberate is that the list lived in
# two places: `cli.py::_parse_require` matched kinds in its own if-chain, and
# `typechecker.py` validates against this set, so `proof_complete` was parseable in
# neither and the two could disagree about the same source. A kind accepted by one and
# refused by the other is a source file the surface cannot describe.
#
# `proof_complete` is the compiler's name for what this surface called `proof`: the
# compiler's kind list has no `proof`, so `require proof verified` is refused there. Both
# spellings are read so a program written against either works, and which one the source
# used is preserved rather than rewritten.
#
# **These names are the compiler's**, and that is a fact a test asserts rather than a comment:
# `compiler/src/parser.rs::REQUIRE_KIND_NAMES` is the list, and `tests/test_surface_drift.py`
# reads it and compares. The two had drifted — this surface knew nine of the eighteen, so
# `require route_score >= 90`, which three shipped examples use and the compiler accepts, was
# refused here as `malformed require 'route_score'` (TICKET-091). A guard kind this surface
# does not *model* is still carried: `runner.rust_intent_envelope` passes `requires` through to
# the compiler verbatim, so refusing one was the surface claiming authority over a list it does
# not own.
REQUIRE_KINDS = {
    'finality',
    'slippage',
    # `require fees <= <bps>` — the ceiling a body relies on, whose declaration half is
    # `risk { max_total_fee_bps N }`. Read by the same rule as every other kind: a kind this
    # surface does not model is still carried, because the envelope passes `requires` through.
    'fees',
    'profit',
    'invariant',
    'risk',
    'nonce',
    'audit_gate',
    'bridge_liquidity',
    'canonical_supply',
    'relayer_quorum',
    'route_score',
    'solver_bond',
    'proof_complete',
    'refund_path',
    'refund_to',
    'finality_explicit',
    'vm_supported',
    'mainnet_safe',
    # The compiler's list has no `proof`; this surface read that spelling before it read
    # `proof_complete`, and both are accepted so a program written against either works. It is
    # the one entry here that is not the compiler's, and the test asserts exactly that.
    'proof',
}

DEX_LIQUIDITY = {
    'raydium': {('USDC', 'SOL'): 200_000.0, ('SOL', 'USDC'): 200_000.0},
    'orca': {('USDC', 'SOL'): 150_000.0},
    'uniswap': {('WSOL', 'USDC'): 500_000.0, ('WETH', 'USDC'): 2_000_000.0},
}

CONTRACT_ADDRESSES = {
    'uniswap': '0x1111111254EEB25477B68fb85Ed929f73A960582',
    'uniswap_v2_router': '0xUniswapRouter000000000000000000000000',
    'raydium': '4k3Dyjzvzp8e2y8bKE8xUrg8rXQZkmh8Y2xS1QZsbJt4',
    'x3': 'x3_bridge_router_v1',
}

TOKEN_ADDRESSES = {
    'ethereum': {
        'USDC': '0xA0b86991c6218b36c1d19d4a2e9eb0ce3606eb48',
        'WSOL': '0x0000000000000000000000000000000000000001',
        'WETH': '0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2',
    },
    'solana': {
        'USDC': 'EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v',
        'SOL': '11111111111111111111111111111111',
        'WSOL': 'So11111111111111111111111111111111111111112',
    },
}

DEFAULT_RECIPIENT = '0x0000000000000000000000000000000000000000'


def normalize_chain(chain):
    if chain is None:
        return None
    return ALIASES.get(str(chain).lower())


def is_valid_receiver(chain, receiver):
    if receiver in (None, '', 'sender'):
        return True
    chain = normalize_chain(chain)
    kind = CHAIN_DOMAINS.get(chain, {}).get('address')
    if kind == 'evm' and chain == 'bsc':
        # The BSC connector addresses real deposits and refunds, never short DSL placeholders.
        return re.fullmatch(r'0x[a-fA-F0-9]{40}', str(receiver)) is not None
    if kind == 'evm':
        # **Any `0x`-prefixed hex**, not exactly forty digits. The compiler validates no
        # address shape at all, so requiring forty here made this surface refuse files the
        # language accepts — `examples/arb_scope.x3` writes `receiver 0xA1` and
        # `examples/intent_fusion.x3` writes `0x1`, both of which the compiler checks and
        # builds (TICKET-091). What is still refused is something that is not an address shape
        # at all, which is the typo this check is for: `not-an-evm-address` has no `0x` and no
        # hex in it. A placeholder short enough to be one is accepted *and* is what the
        # repository's own examples use.
        return re.fullmatch(r'0x[a-fA-F0-9]+', str(receiver)) is not None
    if kind == 'base58':
        return re.fullmatch(r'[1-9A-HJ-NP-Za-km-z]{32,44}', str(receiver)) is not None
    if kind == 'btc':
        return re.fullmatch(r'(bc1|[13])[a-zA-HJ-NP-Z0-9]{25,90}', str(receiver)) is not None
    if kind == 'x3':
        return re.fullmatch(r'(x3_[a-zA-Z0-9_]{6,}|0x[a-fA-F0-9]{40})', str(receiver)) is not None
    if kind == 'near':
        return is_valid_near_account(receiver)
    if kind in UTXO_ADDRESS_RULES:
        return utxo_address_kind(kind, receiver) is not None
    if kind == 'xrp':
        return is_valid_xrp_address(receiver)
    if kind == 'move32':
        return is_valid_move32_address(receiver)
    if kind == 'tron':
        return is_valid_tron_address(receiver)
    if kind == 'ton':
        return is_valid_ton_address(receiver)
    return False


_BASE58 = '123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz'


def base58_decode(text):
    """Bitcoin-alphabet base58 to bytes, leading `1`s as zero bytes. Raises ValueError."""
    value = 0
    for char in text:
        index = _BASE58.find(char)
        if index < 0:
            raise ValueError(f'{char!r} is not base58')
        value = value * 58 + index
    body = value.to_bytes((value.bit_length() + 7) // 8, 'big') if value else b''
    return b'\0' * (len(text) - len(text.lstrip('1'))) + body


def base58_encode(data):
    value = int.from_bytes(data, 'big')
    out = ''
    while value:
        value, rem = divmod(value, 58)
        out = _BASE58[rem] + out
    return '1' * (len(data) - len(data.lstrip(b'\0'))) + out


def tron_address_bytes(address):
    """The 21-byte Tron address (0x41 + 20 bytes) behind a base58check `T…` address, or None."""
    text = str(address)
    if not re.fullmatch(r'T[1-9A-HJ-NP-Za-km-z]{33}', text):
        return None
    try:
        raw = base58_decode(text)
    except ValueError:
        return None
    if len(raw) != 25 or raw[0] != 0x41:
        return None
    if hashlib.sha256(hashlib.sha256(raw[:21]).digest()).digest()[:4] != raw[21:]:
        return None
    return raw[:21]


def tron_address_from_hex(hex21):
    """`41…` (42 hex digits) to the base58check `T…` form."""
    raw = bytes.fromhex(hex21)
    if len(raw) != 21 or raw[0] != 0x41:
        raise ValueError('a Tron address is 0x41 followed by 20 bytes')
    return base58_encode(raw + hashlib.sha256(hashlib.sha256(raw).digest()).digest()[:4])


def is_valid_tron_address(address):
    """A mainnet Tron base58check address with a correct checksum."""
    return tron_address_bytes(address) is not None


# --- UTXO chains -------------------------------------------------------------------------------

def base58check_payload(text):
    """The version+hash bytes behind a base58check string with a correct checksum, or None."""
    if not re.fullmatch(r'[1-9A-HJ-NP-Za-km-z]{25,40}', str(text)):
        return None
    try:
        raw = base58_decode(str(text))
    except ValueError:
        return None
    if len(raw) < 5 or hashlib.sha256(hashlib.sha256(raw[:-4]).digest()).digest()[:4] != raw[-4:]:
        return None
    return raw[:-4]


_BECH32 = 'qpzry9x8gf2tvdw0s3jn54khce6mua7l'
_BECH32_CONST, _BECH32M_CONST = 1, 0x2BC830A3


def _bech32_polymod(values):
    gen = (0x3B6A57B2, 0x26508E6D, 0x1EA119FA, 0x3D4233DD, 0x2A1462B3)
    chk = 1
    for v in values:
        top = chk >> 25
        chk = (chk & 0x1FFFFFF) << 5 ^ v
        for i in range(5):
            chk ^= gen[i] if (top >> i) & 1 else 0
    return chk


def _bech32_hrp_expand(hrp):
    return [ord(c) >> 5 for c in hrp] + [0] + [ord(c) & 31 for c in hrp]


def _convertbits(data, frombits, tobits, pad):
    acc = bits = 0
    out = []
    maxv = (1 << tobits) - 1
    for value in data:
        if value < 0 or value >> frombits:
            return None
        acc = (acc << frombits) | value
        bits += frombits
        while bits >= tobits:
            bits -= tobits
            out.append((acc >> bits) & maxv)
    if pad and bits:
        out.append((acc << (tobits - bits)) & maxv)
    elif not pad and (bits >= frombits or ((acc << (tobits - bits)) & maxv)):
        return None
    return out


def segwit_address_bytes(hrp, address):
    """(witness version, program) for a BIP-173/350 segwit address on `hrp`, or None."""
    text = str(address)
    if text.lower() != text and text.upper() != text:
        return None
    text = text.lower()
    if not text.startswith(hrp + '1') or len(text) > 90:
        return None
    data = []
    for c in text[len(hrp) + 1:]:
        if c not in _BECH32:
            return None
        data.append(_BECH32.find(c))
    if len(data) < 7:
        return None
    const = _bech32_polymod(_bech32_hrp_expand(hrp) + data)
    version = data[0]
    program = _convertbits(data[1:-6], 5, 8, False)
    if program is None or version > 16 or not 2 <= len(program) <= 40:
        return None
    if version == 0 and (const != _BECH32_CONST or len(program) not in (20, 32)):
        return None
    if version > 0 and const != _BECH32M_CONST:
        return None
    return version, bytes(program)


def segwit_address(hrp, version, program):
    data = [version] + _convertbits(list(program), 8, 5, True)
    const = _BECH32_CONST if version == 0 else _BECH32M_CONST
    polymod = _bech32_polymod(_bech32_hrp_expand(hrp) + data + [0] * 6) ^ const
    return hrp + '1' + ''.join(_BECH32[d] for d in data + [(polymod >> 5 * (5 - i)) & 31 for i in range(6)])


def _cashaddr_polymod(values):
    c = 1
    for d in values:
        c0 = c >> 35
        c = ((c & 0x07FFFFFFFF) << 5) ^ d
        for bit, gen in ((1, 0x98F2BC8E61), (2, 0x79B76D99E2), (4, 0xF33E5FB3C4), (8, 0xAE2EABE2A8), (16, 0x1E4F43E470)):
            if c0 & bit:
                c ^= gen
    return c ^ 1


def cashaddr_payload(address, prefix='bitcoincash'):
    """(type, hash) behind a CashAddr address (prefix optional) with a correct checksum, or None."""
    text = str(address)
    if text.lower() != text and text.upper() != text:
        return None
    text = text.lower()
    body = text[len(prefix) + 1:] if text.startswith(prefix + ':') else text
    if ':' in body:
        return None
    try:
        data = [_BECH32.index(c) for c in body]
    except ValueError:
        return None
    if len(data) < 9 or _cashaddr_polymod([ord(c) & 31 for c in prefix] + [0] + data) != 0:
        return None
    payload = _convertbits(data[:-8], 5, 8, False)
    if payload is None or len(payload) != 21 or payload[0] not in (0x00, 0x08):
        # Only 160-bit P2PKH (type 0) and P2SH (type 1) hashes.
        return None
    return ('p2pkh' if payload[0] == 0 else 'p2sh'), bytes(payload[1:])


def cashaddr(kind, hash160, prefix='bitcoincash'):
    payload = _convertbits([0x00 if kind == 'p2pkh' else 0x08] + list(hash160), 8, 5, True)
    checksum = _cashaddr_polymod([ord(c) & 31 for c in prefix] + [0] + payload + [0] * 8)
    return prefix + ':' + ''.join(_BECH32[d] for d in payload + [(checksum >> 5 * (7 - i)) & 31 for i in range(8)])


#: Base58check version bytes per chain, and the segwit hrp where the chain has one. Zcash's
#: are its two-byte transparent prefixes (t1, t3): shielded and unified addresses are refused,
#: because nothing here can prove a payment to one. Bitcoin Cash takes CashAddr only: its legacy
#: form is indistinguishable from a Bitcoin address.
UTXO_ADDRESS_RULES = {
    'btc': {'base58': {b'\x00': 'p2pkh', b'\x05': 'p2sh'}, 'hrp': 'bc'},
    'doge': {'base58': {b'\x1e': 'p2pkh', b'\x16': 'p2sh'}},
    'ltc': {'base58': {b'\x30': 'p2pkh', b'\x32': 'p2sh', b'\x05': 'p2sh'}, 'hrp': 'ltc'},
    'dash': {'base58': {b'\x4c': 'p2pkh', b'\x10': 'p2sh'}},
    'zec': {'base58': {b'\x1c\xb8': 'p2pkh', b'\x1c\xbd': 'p2sh'}},
    'bch': {'cashaddr': 'bitcoincash'},
}


def utxo_address_kind(kind, address):
    """'p2pkh' / 'p2sh' / 'segwit' when `address` is a valid mainnet address for this UTXO chain."""
    rules = UTXO_ADDRESS_RULES.get(kind)
    if rules is None or not isinstance(address, str):
        return None
    if 'cashaddr' in rules:
        found = cashaddr_payload(address, rules['cashaddr'])
        return found[0] if found else None
    if rules.get('hrp') and address.lower().startswith(rules['hrp'] + '1'):
        return 'segwit' if segwit_address_bytes(rules['hrp'], address) else None
    payload = base58check_payload(address)
    if payload is None:
        return None
    for version, kind_name in rules['base58'].items():
        if payload.startswith(version) and len(payload) == len(version) + 20:
            return kind_name
    return None


def canonical_utxo_address(kind, address):
    """One spelling per address, so two renderings of it compare equal (CashAddr, bech32 case)."""
    if kind == 'bch':
        found = cashaddr_payload(address)
        return cashaddr(*found) if found else None
    if UTXO_ADDRESS_RULES[kind].get('hrp') and str(address).lower().startswith(UTXO_ADDRESS_RULES[kind]['hrp'] + '1'):
        return str(address).lower()
    return str(address)


_RIPPLE_BASE58 = 'rpshnaf39wBUDNEGHJKLM4PQRST7VWXYZ2bcdeCg65jkm8oFqi1tuvAxyz'


def is_valid_xrp_address(address):
    """A classic XRP Ledger account address (`r…`, Ripple-alphabet base58check, version 0x00).
    X-addresses are refused: they fold a destination tag in, and this adapter keeps tags explicit."""
    text = str(address)
    if not re.fullmatch(r'r[' + _RIPPLE_BASE58 + r']{24,34}', text):
        return False
    value = 0
    for char in text:
        value = value * 58 + _RIPPLE_BASE58.index(char)
    body = value.to_bytes((value.bit_length() + 7) // 8, 'big') if value else b''
    raw = b'\0' * (len(text) - len(text.lstrip('r'))) + body
    return (len(raw) == 25 and raw[0] == 0x00
            and hashlib.sha256(hashlib.sha256(raw[:21]).digest()).digest()[:4] == raw[21:])


def is_valid_move32_address(address):
    """A full 32-byte Sui/Aptos/Movement account address (`0x` + 64 hex). These chains have no
    address checksum, so the short special forms (`0x1`) are refused: a payout needs the full form."""
    return re.fullmatch(r'0x[0-9a-fA-F]{64}', str(address)) is not None


def is_valid_ton_address(address):
    """A TON mainnet address: raw `<workchain>:<64 hex>` (workchain 0 or -1), or the 48-character
    user-friendly form (base64 or base64url) with a correct CRC16 and no testnet flag."""
    text = str(address)
    if re.fullmatch(r'(0|-1):[0-9a-fA-F]{64}', text):
        return True
    if not re.fullmatch(r'[A-Za-z0-9_\-+/]{48}', text):
        return False
    try:
        raw = base64.urlsafe_b64decode(text.replace('+', '-').replace('/', '_'))
    except (binascii.Error, ValueError):
        return False
    if len(raw) != 36 or raw[0] not in (0x11, 0x51) or raw[1] not in (0x00, 0xFF):
        # 0x11 bounceable, 0x51 non-bounceable; a set 0x80 bit marks a testnet address.
        return False
    return binascii.crc_hqx(raw[:34], 0) == int.from_bytes(raw[34:], 'big')


# NEAR account ID rules (nomicon "Account ID"): 2–64 characters, lowercase alphanumerics
# separated by `-` or `_` within a part and by `.` between parts. A 64-character lowercase hex
# implicit account satisfies the same pattern; an ETH-implicit account is `0x` plus 40 hex.
_NEAR_ACCOUNT_RE = re.compile(r'(([a-z\d]+[-_])*[a-z\d]+\.)*([a-z\d]+[-_])*[a-z\d]+')
_NEAR_ETH_IMPLICIT_RE = re.compile(r'0x[a-f0-9]{40}')


def is_valid_near_account(account):
    text = str(account)
    if _NEAR_ETH_IMPLICIT_RE.fullmatch(text):
        return True
    return 2 <= len(text) <= 64 and _NEAR_ACCOUNT_RE.fullmatch(text) is not None
