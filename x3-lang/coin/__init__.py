"""Author and compile fixed-supply EVM/Solana coin declarations, entirely offline."""
import hashlib
import json
import re

from ed25519 import _valid_public_key as valid_signing_key
from ed25519 import b58decode
from emitter._keccak import keccak_256

MAX_SOURCE_BYTES = 8192
U64_MAX = (1 << 64) - 1
FIELDS = {
    "coin": ("name", "decimals", "cap", "evm", "svm"),
    "evm": ("chain_id", "contract", "recipient", "allocation"),
    "svm": ("genesis_hash", "mint", "recipient", "allocation"),
}


class CoinError(ValueError):
    """A bounded public diagnostic for a rejected declaration."""


def _fields(obj, expected, scope):
    if not isinstance(obj, dict) or obj.keys() != set(expected):
        raise CoinError(f"{scope} requires exactly these fields: {', '.join(expected)}")


def _uint(value, field, limit, minimum=0):
    if not isinstance(value, str) or not re.fullmatch(r"0|[1-9][0-9]{0,77}", value):
        raise CoinError(f"{field} requires an unsigned decimal string without leading zeros")
    number = int(value)
    if not minimum <= number <= limit:
        raise CoinError(f"{field} is outside the supported range {minimum}..{limit}")
    return number


def _address(value, field):
    if not isinstance(value, str) or not re.fullmatch(r"0x[0-9a-fA-F]{40}", value) or int(value, 16) == 0:
        raise CoinError(f"{field} requires a nonzero 20-byte hex address")
    digits = value[2:]
    if digits not in (digits.lower(), digits.upper()):
        digest = keccak_256(digits.lower().encode("ascii")).hex()
        checked = "".join(c.upper() if int(h, 16) >= 8 else c for c, h in zip(digits.lower(), digest))
        if digits != checked:
            raise CoinError(f"{field} has an invalid mixed-case address checksum")
    return value.lower()


def _key(value, field):
    if not isinstance(value, str) or not re.fullmatch(r"[1-9A-HJ-NP-Za-km-z]{32,44}", value):
        raise CoinError(f"{field} requires a full base58-encoded 32-byte value")
    raw = b58decode(value)
    if len(raw) != 32 or raw == bytes(32):
        raise CoinError(f"{field} requires a nonzero 32-byte value")
    if field == "svm.mint" and not valid_signing_key(raw):
        raise CoinError("svm.mint must be a keypair address that can sign account creation")
    return value


def _validate(spec):
    _fields(spec, ("symbol", *FIELDS["coin"]), "coin")
    name, symbol = spec["name"], spec["symbol"]
    if not isinstance(name, str) or not re.fullmatch(r"[A-Za-z0-9](?:[A-Za-z0-9 ._-]{0,30}[A-Za-z0-9])?", name):
        raise CoinError("name must be 1..32 ASCII letters, digits, spaces, dots, underscores or hyphens, with alphanumeric ends")
    if not isinstance(symbol, str) or not re.fullmatch(r"[A-Z][A-Z0-9]{0,11}", symbol):
        raise CoinError("symbol must be 1..12 uppercase ASCII letters or digits, starting with a letter")
    decimals = spec["decimals"]
    if type(decimals) is int and 0 <= decimals <= 18:
        decimals = str(decimals)
    decimals = _uint(decimals, "decimals", 18)
    cap = _uint(spec["cap"], "cap", U64_MAX, 1)
    evm, svm = spec["evm"], spec["svm"]
    _fields(evm, FIELDS["evm"], "evm")
    _fields(svm, FIELDS["svm"], "svm")
    evm = {
        "chain_id": _uint(evm["chain_id"], "evm.chain_id", 2**256 - 1, 1),
        "contract": _address(evm["contract"], "evm.contract"),
        "recipient": _address(evm["recipient"], "evm.recipient"),
        "allocation": _uint(evm["allocation"], "evm.allocation", U64_MAX, 1),
    }
    svm = {
        "genesis_hash": _key(svm["genesis_hash"], "svm.genesis_hash"),
        "mint": _key(svm["mint"], "svm.mint"),
        "recipient": _key(svm["recipient"], "svm.recipient"),
        "allocation": _uint(svm["allocation"], "svm.allocation", U64_MAX, 1),
    }
    if evm["recipient"] == evm["contract"] or svm["recipient"] == svm["mint"]:
        raise CoinError("a genesis recipient must differ from the contract or mint address")
    if evm["allocation"] + svm["allocation"] != cap:
        raise CoinError("evm.allocation + svm.allocation must equal cap in base units")
    return {"symbol": symbol, "name": name, "decimals": decimals, "cap": cap, "evm": evm, "svm": svm}


def _parse(source):
    if not isinstance(source, str) or len(source) > MAX_SOURCE_BYTES or not source.isascii():
        raise CoinError(f"coin source requires ASCII text, at most {MAX_SOURCE_BYTES} bytes")
    if any(ord(c) < 32 and c not in "\n\r\t" or ord(c) == 127 for c in source):
        raise CoinError("coin source contains a control character")
    spec = {}
    stack = []
    for lineno, raw in enumerate(source.splitlines(), 1):
        line = raw.strip()
        if not line or line.startswith("//"):
            continue
        if not spec:
            header = re.fullmatch(r"coin ([A-Z][A-Z0-9]{0,11})\s*\{", line)
            if not header:
                raise CoinError(f"line {lineno}: expected coin SYMBOL {{")
            spec["symbol"] = header[1]
            stack.append("coin")
            continue
        if not stack:
            raise CoinError(f"line {lineno}: trailing content after the coin declaration")
        if line == "}":
            stack.pop()
            continue
        scope = stack[-1]
        target = spec if scope == "coin" else spec[scope]
        field, _, value = line.partition(" ")
        value = value.strip()
        if field not in FIELDS[scope] or field in target:
            raise CoinError(f"line {lineno}: unknown or duplicate field in {scope}")
        if field in ("evm", "svm"):
            if value != "{":
                raise CoinError(f"line {lineno}: expected {field} {{")
            target[field] = {}
            stack.append(field)
        elif field == "name":
            matched = re.fullmatch(r'"([^"]*)"', value)
            if not matched:
                raise CoinError(f"line {lineno}: expected a quoted coin name")
            target[field] = matched[1]
        else:
            target[field] = value
    if stack or not spec:
        raise CoinError("incomplete coin declaration")
    return _validate(spec)


def _manifest(coin):
    # Large numbers cross Python, JSON and JavaScript boundaries without rounding.
    return {
        "format": "x3.coin.fixed-genesis.v1",
        "symbol": coin["symbol"], "name": coin["name"], "decimals": coin["decimals"],
        "cap": str(coin["cap"]),
        "evm": {**coin["evm"], "chain_id": str(coin["evm"]["chain_id"]), "allocation": str(coin["evm"]["allocation"])},
        "svm": {**coin["svm"], "allocation": str(coin["svm"]["allocation"])},
    }


def author_coin(spec):
    """Validate an issuer's structured request and write canonical .x3 source."""
    coin = _validate(spec)
    lines = [f"coin {coin['symbol']} {{", f'  name "{coin["name"]}"', f"  decimals {coin['decimals']}", f"  cap {coin['cap']}"]
    for target in ("evm", "svm"):
        lines.append(f"  {target} {{")
        lines.extend(f"    {key} {coin[target][key]}" for key in FIELDS[target])
        lines.append("  }")
    return "\n".join([*lines, "}", ""])


def compile_coin(source):
    """Compile one .x3 declaration to a reproducible, unsigned issuance bundle."""
    from .evm import render_evm
    from .svm import build_svm

    coin = _parse(source)
    manifest = _manifest(coin)
    encoded = json.dumps(manifest, sort_keys=True, separators=(",", ":"), ensure_ascii=True).encode("ascii")
    digest = hashlib.sha256(encoded).hexdigest()
    coin["declaration_hash"] = digest
    return {
        "manifest": manifest, "declaration_hash": digest,
        "source_sha256": hashlib.sha256(source.encode("ascii")).hexdigest(),
        "evm_solidity": render_evm(coin), "svm_issuance": build_svm(coin),
    }
