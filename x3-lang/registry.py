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
REQUIRE_KINDS = {'finality', 'slippage', 'profit', 'nonce', 'proof', 'bridge_liquidity', 'canonical_supply', 'invariant'}

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
        return re.fullmatch(r'0x[a-fA-F0-9]{40}', str(receiver)) is not None
    if kind == 'base58':
        return re.fullmatch(r'[1-9A-HJ-NP-Za-km-z]{32,44}', str(receiver)) is not None
    if kind == 'btc':
        return re.fullmatch(r'(bc1|[13])[a-zA-HJ-NP-Z0-9]{25,90}', str(receiver)) is not None
    if kind == 'x3':
        return re.fullmatch(r'(x3_[a-zA-Z0-9_]{6,}|0x[a-fA-F0-9]{40})', str(receiver)) is not None
    return False
