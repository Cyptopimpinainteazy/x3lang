"""Ed25519 (RFC 8032) and Base58, for verifying 1Click quote signatures.

Verification uses the `cryptography` package (OpenSSL) when it is installed. Without it, a
pure-Python RFC 8032 verifier is used. That fallback is the RFC's own reference algorithm,
and it is checked against the RFC's test vectors when this module is imported. If the check
fails, the import fails. Verification never silently degrades to "accept".

`sign` exists for the test suite, which signs synthetic quotes with a throwaway key. It is
the RFC reference signer, and nothing in the adapter's production path calls it.
"""
import hashlib
from typing import Optional, Tuple

__all__ = ["verify", "sign", "public_key", "b58decode", "b58encode", "BACKEND"]

# --------------------------------------------------------------------------------------------
# Base58 (Bitcoin alphabet: the one NEAR keys, signatures and 1Click hashes use)
# --------------------------------------------------------------------------------------------

_B58_ALPHABET = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"
_B58_INDEX = {c: i for i, c in enumerate(_B58_ALPHABET)}


def b58encode(data: bytes) -> str:
    n = int.from_bytes(data, "big")
    out = []
    while n:
        n, r = divmod(n, 58)
        out.append(_B58_ALPHABET[r])
    pad = len(data) - len(data.lstrip(b"\0"))
    return "1" * pad + "".join(reversed(out))


def b58decode(text: str) -> bytes:
    if not isinstance(text, str) or not text:
        raise ValueError("base58 input must be a non-empty string")
    n = 0
    for c in text:
        if c not in _B58_INDEX:
            raise ValueError(f"invalid base58 character {c!r}")
        n = n * 58 + _B58_INDEX[c]
    body = n.to_bytes((n.bit_length() + 7) // 8, "big") if n else b""
    pad = len(text) - len(text.lstrip("1"))
    return b"\0" * pad + body


# --------------------------------------------------------------------------------------------
# RFC 8032 section 6 reference implementation
# --------------------------------------------------------------------------------------------

_P = 2 ** 255 - 19
_D = -121665 * pow(121666, _P - 2, _P) % _P
_Q = 2 ** 252 + 27742317777372353535851937790883648493
_SQRT_M1 = pow(2, (_P - 1) // 4, _P)

Point = Tuple[int, int, int, int]


def _sha512(data: bytes) -> bytes:
    return hashlib.sha512(data).digest()


def _sha512_modq(data: bytes) -> int:
    return int.from_bytes(_sha512(data), "little") % _Q


def _add(P: Point, Q: Point) -> Point:
    A = (P[1] - P[0]) * (Q[1] - Q[0]) % _P
    B = (P[1] + P[0]) * (Q[1] + Q[0]) % _P
    C = 2 * P[3] * Q[3] * _D % _P
    D = 2 * P[2] * Q[2] % _P
    E, F, G, H = B - A, D - C, D + C, B + A
    return (E * F % _P, G * H % _P, F * G % _P, E * H % _P)


def _mul(s: int, P: Point) -> Point:
    Q: Point = (0, 1, 1, 0)
    while s > 0:
        if s & 1:
            Q = _add(Q, P)
        P = _add(P, P)
        s >>= 1
    return Q


def _equal(P: Point, Q: Point) -> bool:
    if (P[0] * Q[2] - Q[0] * P[2]) % _P != 0:
        return False
    return (P[1] * Q[2] - Q[1] * P[2]) % _P == 0


def _recover_x(y: int, sign_bit: int) -> Optional[int]:
    if y >= _P:
        return None
    x2 = (y * y - 1) * pow(_D * y * y + 1, _P - 2, _P)
    if x2 == 0:
        return None if sign_bit else 0
    x = pow(x2, (_P + 3) // 8, _P)
    if (x * x - x2) % _P != 0:
        x = x * _SQRT_M1 % _P
    if (x * x - x2) % _P != 0:
        return None
    if (x & 1) != sign_bit:
        x = _P - x
    return x


_G_Y = 4 * pow(5, _P - 2, _P) % _P
_G_X = _recover_x(_G_Y, 0)
_G: Point = (_G_X, _G_Y, 1, _G_X * _G_Y % _P)


def _compress(P: Point) -> bytes:
    zinv = pow(P[2], _P - 2, _P)
    x, y = P[0] * zinv % _P, P[1] * zinv % _P
    return int.to_bytes(y | ((x & 1) << 255), 32, "little")


def _decompress(s: bytes) -> Optional[Point]:
    if len(s) != 32:
        return None
    y = int.from_bytes(s, "little")
    sign_bit = y >> 255
    y &= (1 << 255) - 1
    x = _recover_x(y, sign_bit)
    if x is None:
        return None
    return (x, y, 1, x * y % _P)


def _valid_public_key(public: bytes) -> bool:
    """A canonical, non-identity point in the prime-order Ed25519 subgroup.

    OpenSSL's Ed25519 verifier accepts the identity point and can then accept
    the universal forgery R=B, S=1 for any message. The reference verifier has
    the same mathematical acceptance unless the decoded key is constrained.
    This check runs before either backend and rejects small-order points and
    points carrying a torsion component.
    """
    if type(public) is not bytes or len(public) != 32:
        return False
    point = _decompress(public)
    if point is None or _compress(point) != public:
        return False
    identity: Point = (0, 1, 1, 0)
    return not _equal(point, identity) and _equal(_mul(_Q, point), identity)


def _expand(secret: bytes) -> Tuple[int, bytes]:
    if len(secret) != 32:
        raise ValueError("Ed25519 secret key seed must be 32 bytes")
    h = _sha512(secret)
    a = int.from_bytes(h[:32], "little")
    a &= (1 << 254) - 8
    a |= 1 << 254
    return a, h[32:]


def public_key(secret: bytes) -> bytes:
    a, _ = _expand(secret)
    return _compress(_mul(a, _G))


def sign(secret: bytes, message: bytes) -> bytes:
    a, prefix = _expand(secret)
    A = _compress(_mul(a, _G))
    r = _sha512_modq(prefix + message)
    Rs = _compress(_mul(r, _G))
    h = _sha512_modq(Rs + A + message)
    s = (r + h * a) % _Q
    return Rs + int.to_bytes(s, 32, "little")


def _verify_reference(public: bytes, message: bytes, signature: bytes) -> bool:
    if len(public) != 32 or len(signature) != 64:
        return False
    if not _valid_public_key(public):
        return False
    A = _decompress(public)
    if A is None:
        return False
    Rs = signature[:32]
    R = _decompress(Rs)
    if R is None:
        return False
    s = int.from_bytes(signature[32:], "little")
    if s >= _Q:
        return False
    h = _sha512_modq(Rs + public + message)
    return _equal(_mul(s, _G), _add(R, _mul(h, A)))


# RFC 8032 section 7.1, TEST 1 and TEST 2.
_VECTORS = (
    (
        "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60",
        "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a",
        "",
        "e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e065224901555fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b",
    ),
    (
        "4ccd089b28ff96da9db6c346ec114e0f5b8a319f35aba624da8cf6ed4fb8a6fb",
        "3d4017c3e843895a92b70aa74d1b7ebc9c982ccf2ec4968cc0cd55f12af4660c",
        "72",
        "92a009a9f0d4cab8720e820b5f642540a2b27b5416503f8fb3762223ebdb69da085ac1e43e15996e458f3613d0f11d8c387b2eaeb4302aeeb00d291612bb0c00",
    ),
)


def _self_test() -> None:
    for secret, public, message, signature in _VECTORS:
        sk, pk, msg, sig = (bytes.fromhex(x) for x in (secret, public, message, signature))
        if public_key(sk) != pk or sign(sk, msg) != sig:
            raise ImportError("ed25519: RFC 8032 test vector mismatch in the reference signer")
        if not _verify_reference(pk, msg, sig):
            raise ImportError("ed25519: RFC 8032 test vector rejected by the reference verifier")
        if _verify_reference(pk, msg + b"\0", sig):
            raise ImportError("ed25519: reference verifier accepted an altered message")


_self_test()

try:
    from cryptography.exceptions import InvalidSignature as _InvalidSignature
    from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey as _Ed25519PublicKey

    def _verify_openssl(public: bytes, message: bytes, signature: bytes) -> bool:
        if len(public) != 32 or len(signature) != 64:
            return False
        try:
            _Ed25519PublicKey.from_public_bytes(public).verify(signature, message)
            return True
        except (_InvalidSignature, ValueError):
            return False

    for _sk, _pk, _msg, _sig in _VECTORS:
        if not _verify_openssl(bytes.fromhex(_pk), bytes.fromhex(_msg), bytes.fromhex(_sig)):
            raise ImportError("ed25519: cryptography rejected an RFC 8032 test vector")
    _verify = _verify_openssl
    BACKEND = "cryptography"
except ImportError as exc:
    if "ed25519:" in str(exc):
        raise
    _verify = _verify_reference
    BACKEND = "rfc8032-reference"


def verify(public: bytes, message: bytes, signature: bytes) -> bool:
    """True only for a valid Ed25519 signature of `message` under the 32-byte `public` key."""
    public = bytes(public)
    message = bytes(message)
    signature = bytes(signature)
    if not _valid_public_key(public):
        return False
    return _verify(public, message, signature)
