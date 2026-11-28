"""Offline syntax checks recovered from the historical X3 route branch.

Each profile recognizes a particular encoding. Profiles may overlap across
networks and cannot prove ownership, account existence or payment eligibility.
"""
import argparse
import base64
import binascii
import hashlib
import json
import re

from ed25519 import b58decode


def _base58check_payload(text):
    try:
        raw = b58decode(text)
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


def _decode_5bit_groups(data):
    # Unpadded base32 groups may end with at most four zero bits.
    frombits, tobits = 5, 8
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
    if bits >= frombits or ((acc << (tobits - bits)) & maxv):
        return None
    return out


def _segwit_address_bytes(hrp, address):
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
    program = _decode_5bit_groups(data[1:-6])
    if program is None or version > 16 or not 2 <= len(program) <= 40:
        return None
    if version == 0 and (const != _BECH32_CONST or len(program) not in (20, 32)):
        return None
    if version > 0 and const != _BECH32M_CONST:
        return None
    return version, bytes(program)


def _cashaddr_polymod(values):
    c = 1
    for d in values:
        c0 = c >> 35
        c = ((c & 0x07FFFFFFFF) << 5) ^ d
        for bit, gen in ((1, 0x98F2BC8E61), (2, 0x79B76D99E2), (4, 0xF33E5FB3C4), (8, 0xAE2EABE2A8), (16, 0x1E4F43E470)):
            if c0 & bit:
                c ^= gen
    return c ^ 1


def _cashaddr_payload(address, prefix='bitcoincash'):
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
    payload = _decode_5bit_groups(data[:-8])
    if payload is None or len(payload) != 21 or payload[0] not in (0x00, 0x08):
        # Only 160-bit P2PKH (type 0) and P2SH (type 1) hashes.
        return None
    return ('p2pkh' if payload[0] == 0 else 'p2sh'), bytes(payload[1:])


UTXO_ADDRESS_RULES = {
    'btc': {'base58': {b'\x00': 'p2pkh', b'\x05': 'p2sh'}, 'hrp': 'bc'},
    'doge': {'base58': {b'\x1e': 'p2pkh', b'\x16': 'p2sh'}},
    'ltc': {'base58': {b'\x30': 'p2pkh', b'\x32': 'p2sh', b'\x05': 'p2sh'}, 'hrp': 'ltc'},
    'dash': {'base58': {b'\x4c': 'p2pkh', b'\x10': 'p2sh'}},
    'zec-transparent': {'base58': {b'\x1c\xb8': 'p2pkh', b'\x1c\xbd': 'p2sh'}},
    'bch-cashaddr': {'cashaddr': 'bitcoincash'},
}


def _utxo_address_kind(kind, address):
    """'p2pkh' / 'p2sh' / 'segwit' when `address` is a valid mainnet address for this UTXO chain."""
    rules = UTXO_ADDRESS_RULES.get(kind)
    if rules is None or not isinstance(address, str):
        return None
    if 'cashaddr' in rules:
        found = _cashaddr_payload(address, rules['cashaddr'])
        return found[0] if found else None
    if rules.get('hrp') and address.lower().startswith(rules['hrp'] + '1'):
        return 'segwit' if _segwit_address_bytes(rules['hrp'], address) else None
    payload = _base58check_payload(address)
    if payload is None:
        return None
    for version, kind_name in rules['base58'].items():
        if payload.startswith(version) and len(payload) == len(version) + 20:
            return kind_name
    return None


_RIPPLE_BASE58 = 'rpshnaf39wBUDNEGHJKLM4PQRST7VWXYZ2bcdeCg65jkm8oFqi1tuvAxyz'


def _xrp_classic(address):
    """A classic XRP Ledger account address (`r…`, Ripple-alphabet base58check, version 0x00).
    X-addresses carry a tag and use a different encoding."""
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


def _stellar_key(address):
    """The 32-byte ed25519 key of a Stellar account (`G…` strkey), or None.

    A strkey is base32 of: version byte (6 << 3 for an account), 32 key bytes, and a CRC16-XModem
    of those 33 bytes, little-endian. This profile covers uppercase G-addresses."""
    text = str(address)
    if not re.fullmatch(r'G[A-Z2-7]{55}', text):
        return None
    try:
        raw = base64.b32decode(text)
    except (binascii.Error, ValueError):
        return None
    if len(raw) != 35 or raw[0] != 6 << 3:
        return None
    if binascii.crc_hqx(raw[:33], 0) != int.from_bytes(raw[33:], 'little'):
        return None
    return raw[1:33]


def _tron_base58(text):
    if not re.fullmatch(r'T[1-9A-HJ-NP-Za-km-z]{33}', text):
        return False
    payload = _base58check_payload(text)
    return payload is not None and len(payload) == 21 and payload[0] == 0x41


def _ton_friendly(text):
    # TEP-2 allows either standard or URL-safe Base64, without mixing alphabets.
    if not re.fullmatch(r'[A-Za-z0-9+/]{48}|[A-Za-z0-9_-]{48}', text):
        return False
    raw = base64.b64decode(text, altchars=b'-_', validate=True)
    return (raw[0] in (0x11, 0x51) and raw[1] in (0x00, 0xFF)
            and binascii.crc_hqx(raw[:34], 0) == int.from_bytes(raw[34:], 'big'))


PROFILES = tuple(sorted((*UTXO_ADDRESS_RULES, 'tron-base58', 'xrp-classic', 'stellar-g',
                         'ton-friendly', 'ton-raw', 'move32')))


def matches_format(profile, address):
    """Return whether a string matches the selected encoding. Unknown profiles raise ValueError."""
    if profile not in PROFILES:
        raise ValueError('unknown address format')
    # The longest accepted envelope is a 90-character SegWit address.
    if not isinstance(address, str) or not address.isascii() or not 1 <= len(address) <= 90:
        return False
    if profile in UTXO_ADDRESS_RULES:
        return _utxo_address_kind(profile, address) is not None
    if profile == 'stellar-g':
        return _stellar_key(address) is not None
    if profile == 'xrp-classic':
        return _xrp_classic(address)
    if profile == 'tron-base58':
        return _tron_base58(address)
    if profile == 'ton-friendly':
        return _ton_friendly(address)
    if profile == 'ton-raw':
        return re.fullmatch(r'(0|-1):[0-9a-fA-F]{64}', address) is not None
    return re.fullmatch(r'0x[0-9a-fA-F]{64}', address) is not None


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument('profile', choices=PROFILES)
    parser.add_argument('address')
    args = parser.parse_args(argv)
    valid = matches_format(args.profile, args.address)
    print(json.dumps({'profile': args.profile, 'matches_format': valid}))
    return 0 if valid else 1


if __name__ == '__main__':
    raise SystemExit(main())
