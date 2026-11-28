"""Public format vectors and regressions from the historical route branch."""
import json

import pytest

import address_format as af


# Fixed vectors (no encoder under test produces them). The integration guide
# links BIP-350, CashAddr, SEP-23, TEP-2, TRON, XRP and Zcash source examples.
CHECKSUMMED = [
    ('btc', '1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa'),
    ('btc', 'BC1QW508D6QEJXTDG4Y5R3ZARVARY0C5XW7KV8F3T4'),
    ('btc', 'bc1p0xlxvlhemja6c4dqv22uapctqupfhlxm9h8z3k2e72q4k9hcz7vqzk5jj0'),
    ('ltc', 'LaMT348PWRnrqeeWArpwQPbuanpXDZGEUz'),
    ('ltc', 'ltc1qg42tkwuuxefutzxezdkdel39gfstuap288mfea'),
    ('doge', 'DH5yaieqoZN36fDVciNyRueRGvGLR3mr7L'),
    ('dash', 'XpESxaUmonkq8RaLLp46Brx2K39ggQe226'),
    ('zec-transparent', 't3LmX1cxWPPPqL4TZHx42HU3U5ghbFjRiif'),
    ('zec-transparent', 't1KRqwQhktLV4BjbNLiuH6pb3AMoszZKcQB'),  # original t1 vector, 20 bytes of 0x11
    ('bch-cashaddr', 'bitcoincash:qpm2qsznhks23z7629mms6s4cwef74vcwvy22gdx6a'),
    ('bch-cashaddr', 'qpm2qsznhks23z7629mms6s4cwef74vcwvy22gdx6a'),
    ('xrp-classic', 'r9cZA1mLK5R5Am25ArfXFmqgNwjZgnfk59'),
    ('stellar-g', 'GA7QYNF7SOWQ3GLR2BGMZEHXAVIRZA4KVWLTJJFC7MGXUA74P7UJVSGZ'),
    ('tron-base58', 'TNPeeaaFB7K9cmo4uQpcU32zGK8G1NYqeL'),
    ('ton-friendly', 'EQDKbjIcfM6ezt8KjKJJLshZJJSqX7XOA4ff-W72r5gqPrHF'),
    ('ton-friendly', 'UQDKbjIcfM6ezt8KjKJJLshZJJSqX7XOA4ff+W72r5gqPuwA'),
]


@pytest.mark.parametrize('profile,address', CHECKSUMMED)
def test_published_encoding_and_checksum(profile, address):
    assert af.matches_format(profile, address)
    changed = address[:-1] + ('3' if address[-1] == '2' else '2')
    assert not af.matches_format(profile, changed)


@pytest.mark.parametrize('profile,address', [
    ('ltc', '1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa'),
    ('doge', 'LaMT348PWRnrqeeWArpwQPbuanpXDZGEUz'),
    ('dash', 'DH5yaieqoZN36fDVciNyRueRGvGLR3mr7L'),
    ('bch-cashaddr', '1BpEi6DfDAUFd7GtittLSdBeYJvcoaVggu'),
    ('bch-cashaddr', 'bchtest:qpm2qsznhks23z7629mms6s4cwef74vcwvy22gdx6a'),
    ('bch-cashaddr', 'bitcoincash:Qpm2qsznhks23z7629mms6s4cwef74vcwvy22gdx6a'),
    ('zec-transparent', 'zs1' + 'q' * 75),
    ('btc', 'BC1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4'),
    ('btc', 'bc1p0xlxvlhemja6c4dqv22uapctqupfhlxm9h8z3k2e72q4k9hcz7vqh2y7hd'),
    ('btc', 'bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kemeawh'),
    ('btc', 'bc1pw5dgrnzv'),
    ('xrp-classic', '1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa'),
    ('stellar-g', 'gaaaaa' + 'a' * 50),
    ('stellar-g', 'M' + 'A' * 68),
    ('tron-base58', '41' + 'ab' * 20),
    ('ton-friendly', 'kQDKbjIcfM6ezt8KjKJJLshZJJSqX7XOA4ff-W72r5gqPgpP'),
    ('ton-friendly', '0:' + 'ab' * 32),
    ('ton-raw', '5:' + 'ab' * 32),
    ('move32', '0x1'),
    ('move32', '0x' + 'G0' * 32),
])
def test_incompatible_encoding_is_refused(profile, address):
    assert not af.matches_format(profile, address)


def test_shared_bitcoin_and_litecoin_p2sh_version_is_ambiguous():
    address = '3J98t1WpEZ73CNmQviecrnyiWrnqRhWNLy'
    assert af.matches_format('btc', address)
    assert af.matches_format('ltc', address)


@pytest.mark.parametrize('profile,address', [('ton-raw', '-1:' + 'a1' * 32),
                                             ('ton-raw', '0:' + 'a1' * 32),
                                             ('move32', '0x' + 'a1' * 32)])
def test_raw_formats_check_shape_and_have_no_checksum(profile, address):
    assert af.matches_format(profile, address)
    assert af.matches_format(profile, address[:-1] + '2')


@pytest.mark.parametrize('address', [None, 123, [], '', 'Ａ' * 34, '1' * 10_000])
def test_nonstrings_nonascii_and_oversized_values_fail(address):
    assert all(not af.matches_format(profile, address) for profile in af.PROFILES)


def test_cli_and_unknown_format(capsys):
    profile, address = CHECKSUMMED[0]
    assert af.main([profile, address]) == 0
    assert json.loads(capsys.readouterr().out) == {'profile': profile, 'matches_format': True}
    assert af.main([profile, 'invalid']) == 1
    assert not json.loads(capsys.readouterr().out)['matches_format']
    assert af.main(['ton-raw', '--', '-1:' + 'a1' * 32]) == 0
    assert json.loads(capsys.readouterr().out)['matches_format']
    with pytest.raises(ValueError, match='unknown address format'):
        af.matches_format('unknown', address)
    with pytest.raises(SystemExit) as error:
        af.main(['unknown', address])
    assert error.value.code == 2
