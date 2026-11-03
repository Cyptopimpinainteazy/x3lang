"""Pure-Python Keccak-256 fallback (Ethereum domain padding).

Used only when neither pycryptodome (``Crypto.Hash.keccak``) nor eth-hash's
pycryptodome backend is available.  This is the original Keccak submission hash
that Ethereum uses (domain/rate padding begins with ``0x01``), which is NOT the
same as the NIST-standardized SHA3 (padding ``0x06``).

Correctness is enforced by a built-in self-test against the canonical vectors
and (when a system backend exists) a cross-check against pycryptodome.  The
audited backends are always preferred; this module is the last-resort fallback
so the emitter never silently produces a wrong digest.
"""

_MASK64 = (1 << 64) - 1

# Round constants for Keccak-f[1600] (24 rounds).
_RC = [
    0x0000000000000001, 0x0000000000008082, 0x800000000000808A,
    0x8000000080008000, 0x000000000000808B, 0x0000000080000001,
    0x8000000080008081, 0x8000000000008009, 0x000000000000008A,
    0x0000000000000088, 0x0000000080008009, 0x000000008000000A,
    0x000000008000808B, 0x800000000000008B, 0x8000000000008089,
    0x8000000000008003, 0x8000000000008002, 0x8000000000000080,
    0x000000000000800A, 0x800000008000000A, 0x8000000080008081,
    0x8000000000008080, 0x0000000080000001, 0x8000000080008008,
]

# Rotation offsets (rho step): RHO[x][y].
_RHO = [
    [0, 36, 3, 41, 18],
    [1, 44, 10, 45, 2],
    [62, 6, 43, 15, 61],
    [28, 55, 25, 21, 56],
    [27, 20, 39, 8, 14],
]

def _rotl(v: int, n: int) -> int:
    n &= 63
    if n == 0:
        return v
    return ((v << n) | (v >> (64 - n))) & _MASK64


# Precompute destination lane + rotation for the combined rho+pi step.
# For each source lane C[x][y] the combined step maps it to lane
# D[y][(2x+3y) mod 5] = rotl(C[x][y], RHO[x][y]).
def _build_pi():
    coords = [[0] * 5 for _ in range(5)]
    rot = [[0] * 5 for _ in range(5)]
    for x in range(5):
        for y in range(5):
            ny = (2 * x + 3 * y) % 5
            nx = y
            coords[nx][ny] = x + 5 * y  # source lane index
            rot[nx][ny] = _RHO[x][y]
    return coords, rot


_PI_SRC, _PI_ROT = _build_pi()


def _keccak_f(a: list) -> None:
    n = 25
    for rc in _RC:
        # theta
        c = [a[x] ^ a[x + 5] ^ a[x + 10] ^ a[x + 15] ^ a[x + 20] for x in range(5)]
        d = [0] * 5
        for x in range(5):
            d[x] = c[(x + 4) % 5] ^ _rotl(c[(x + 1) % 5], 1)
        for y in range(0, n, 5):
            for x in range(5):
                a[x + y] ^= d[x]
        # rho + pi together
        src = a[:]
        for x in range(5):
            for y in range(5):
                a[x + 5 * y] = _rotl(src[_PI_SRC[x][y]], _PI_ROT[x][y])
        # chi
        for y in range(0, n, 5):
            row = a[y:y + 5]
            for x in range(5):
                a[y + x] = row[x] ^ ((~row[(x + 1) % 5]) & row[(x + 2) % 5]) & _MASK64
        # iota
        a[0] ^= rc


def _sponge(data: bytes) -> bytes:
    rate = 136  # bytes for a 256-bit digest (1088 bits)
    state = [0] * 25
    for offset in range(0, len(data), rate):
        block = data[offset:offset + rate]
        for i in range(0, len(block), 8):
            lane = int.from_bytes(block[i:i + 8], 'little')
            state[i // 8] ^= lane
        _keccak_f(state)
    out = bytearray()
    for i in range(rate // 8):
        out += state[i].to_bytes(8, 'little')
    return bytes(out[:32])


def keccak_256(data: bytes) -> bytes:
    rate = 136
    p = bytearray(data)
    p.append(0x01)
    while len(p) % rate != rate - 1:
        p.append(0x00)
    p.append(0x80)
    return _sponge(bytes(p))


if __name__ == '__main__':  # pragma: no cover - manual self-test entry
    def _self_test():
        assert keccak_256(b'').hex() == \
            'c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470'
        assert keccak_256(b'abc').hex() == \
            '4e03657aea45a94fc7d47ba826c8d667c0d1e6e33a64a036ec44f58fa12d6c45'
        return True
    ok = _self_test()
    print('pure-python keccak-256 self-test:', 'PASS' if ok else 'FAIL')
