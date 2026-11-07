"""Adversarial checks for the shared Ed25519 verifier.

The identity public key plus the universal signature (R=B, S=1) verifies as a
pure group equation for every message. Both OpenSSL and the RFC reference
implementation must reject it before entering the signature backend.
"""
import hashlib

import pytest

import ed25519


IDENTITY = bytes.fromhex(
    "0100000000000000000000000000000000000000000000000000000000000000"
)


def universal_signature() -> bytes:
    point_b = bytes.fromhex("58" + "66" * 31)
    scalar_one = b"\x01" + b"\x00" * 31
    return point_b + scalar_one


@pytest.mark.parametrize(
    "encoded",
    (
        "0000000000000000000000000000000000000000000000000000000000000000",
        "0000000000000000000000000000000000000000000000000000000000000080",
        "0100000000000000000000000000000000000000000000000000000000000000",
        "ecffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff7f",
        "26e8958fc2b227b045c3f489f2ef98f0d5dfac05d3c63339b13802886d53fc05",
        "c7176a703d4dd84fba3c0b760d10670f2a2053fa2c39ccdefb2c3f5d0c3b6e",
    ),
)
def test_small_order_public_keys_are_rejected(encoded):
    assert not ed25519._valid_public_key(bytes.fromhex(encoded))
    assert not ed25519.verify(bytes.fromhex(encoded), b"message", universal_signature())


def test_identity_key_cannot_accept_the_universal_forgery():
    message = b"an approval for every possible checkpoint"
    forgery = universal_signature()
    assert not ed25519.verify(IDENTITY, message, forgery)
    assert not ed25519._verify_reference(IDENTITY, message, forgery)


def test_ordinary_rfc8032_key_remains_valid():
    private = hashlib.sha256(b"strict-ed25519-test").digest()
    public = ed25519.public_key(private)
    message = b"checkpoint approval"
    signature = ed25519.sign(private, message)
    assert ed25519._valid_public_key(public)
    assert ed25519.verify(public, message, signature)
