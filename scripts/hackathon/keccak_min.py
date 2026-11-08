"""Dependency-free Keccak-256 for canonical Ethereum selectors and event topics.

Tested against the standard ERC20 Transfer topic and transfer selector.
"""
from __future__ import annotations

MASK = (1 << 64) - 1
ROT = (
    (0, 36, 3, 41, 18),
    (1, 44, 10, 45, 2),
    (62, 6, 43, 15, 61),
    (28, 55, 25, 21, 56),
    (27, 20, 39, 8, 14),
)
RC = (
  0x0000000000000001, 0x0000000000008082,
  0x800000000000808a, 0x8000000080008000,
  0x000000000000808b, 0x0000000080000001,
  0x8000000080008081, 0x8000000000008009,
  0x000000000000008a, 0x0000000000000088,
  0x0000000080008009, 0x000000008000000a,
  0x000000008000808b, 0x800000000000008b,
  0x8000000000008089, 0x8000000000008003,
  0x8000000000008002, 0x8000000000000080,
  0x000000000000800a, 0x800000008000000a,
  0x8000000080008081, 0x8000000000008080,
  0x0000000080000001, 0x8000000080008008,
)

def _rotl(x: int, n: int) -> int:
    return ((x << n) | (x >> (64-n))) & MASK if n else x

def _permute(a: list[int]) -> None:
    for rc in RC:
        c = [0] * 5
        for x in range(5):
            c[x] = a[x] ^ a[x+5] ^ a[x+10] ^ a[x+15] ^ a[x+20]
        for x in range(5):
            d = c[(x-1)%5] ^ _rotl(c[(x+1)%5],1)
            for y in range(5):
                a[x+5*y] ^= d
        b = [0] * 25
        for x in range(5):
            for y in range(5):
                b[y + 5*((2*x+3*y)%5)] = _rotl(a[x+5*y], ROT[x][y])
        for x in range(5):
            for y in range(5):
                a[x+5*y] = b[x+5*y] ^ ((~b[(x+1)%5+5*y]) & b[(x+2)%5+5*y])
        a[0] ^= rc

def keccak256(data: bytes) -> bytes:
    rate = 136
    padded = data + b'\x01'
    padded += b'\x00' * ((-len(padded)-1) % rate)
    padded += b'\x80'
    state = [0] * 25
    for off in range(0,len(padded),rate):
        block = padded[off:off+rate]
        for i in range(rate//8):
            state[i] ^= int.from_bytes(block[8*i:8*i+8], 'little')
        _permute(state)
    return b''.join(x.to_bytes(8,'little') for x in state)[:32]

if __name__ == '__main__':
    assert keccak256(b'transfer(address,uint256)').hex()[:8] == 'a9059cbb'
    assert keccak256(b'Transfer(address,address,uint256)').hex() == 'ddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef'
    print('Ethereum keccak self-check PASS')
