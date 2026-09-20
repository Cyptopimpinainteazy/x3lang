"""Production registries for chains, assets, DEXs, bridges and address validation."""
import re

CHAIN_DOMAINS = {
    'solana': {'aliases': {'sol', 'svm'}, 'address': 'base58'},
    'ethereum': {'aliases': {'eth', 'evm'}, 'address': 'evm'},
    'arbitrum': {'aliases': {'arb'}, 'address': 'evm'},
    'polygon': {'aliases': {'matic'}, 'address': 'evm'},
    'bitcoin': {'aliases': {'btc'}, 'address': 'btc'},
    'x3': {'aliases': set(), 'address': 'x3'},
}

ALIASES = {alias: chain for chain, meta in CHAIN_DOMAINS.items() for alias in meta['aliases']}
ALIASES.update({chain: chain for chain in CHAIN_DOMAINS})

ASSETS_BY_CHAIN = {
    'solana': {'USDC', 'SOL', 'WSOL', 'USDT', 'BONK'},
    'ethereum': {'USDC', 'USDT', 'WETH', 'ETH', 'WSOL', 'DAI'},
    'arbitrum': {'USDC', 'USDT', 'WETH', 'ETH', 'ARB'},
    'polygon': {'USDC', 'USDT', 'WETH', 'MATIC', 'DAI'},
    'bitcoin': {'BTC', 'WBTC'},
    'x3': {'SOL', 'USDC', 'WSOL', 'WETH', 'BTC', 'X3'},
}

KNOWN_DEXS = {'raydium', 'orca', 'uniswap', 'sushiswap', 'curve'}
KNOWN_BRIDGES = {'x3', 'wormhole', 'layerzero', 'axelar'}
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
    return False
