"""Tests for complementary mandate bots."""
import tempfile
from pathlib import Path

import pytest

from arbitrage.trading_store import TradingStore
from arbitrage.mandate_proof import MandateProof
from arbitrage.cycle_closer import CycleCloser
from arbitrage.fee_splitter import FeeSplitter
from arbitrage.liquidity_listing import LiquidityListing
from arbitrage.mandate_diff import MandateDiff
from arbitrage.trader import Trader


def test_mandate_proof_roundtrip():
    with tempfile.TemporaryDirectory() as d:
        store = TradingStore(Path(d) / 't.sqlite')
        # minimal registered mandate
        compiled = {
            'owner': 'owner.near',
            'agent': 'agent.near',
            'terms': {
                'asset': 'Near.USDC',
                'max_incomplete_cycle_notional': 100_000_000,
                'recipients': ['settlement.near'],
                'assets': ['Near.USDC'],
                'expires_at': 9999999999,
            }
        }
        store._connect().execute(
            'INSERT INTO trade_mandates(id,document) VALUES(?,?)',
            ('mid1', '{"compiled": ' + str(compiled).replace("'", '"') + '}')
        )
        proof = MandateProof(store)
        snap = proof.snapshot('mid1', 1_700_000_000_000)
        assert proof.verify(snap)
        assert int(snap['remaining_incomplete_notional']) >= 0


def test_fee_splitter_positive_only():
    splitter = FeeSplitter()
    completed = {'phase': 'COMPLETED', 'paper_profit_units': '1000000', 'profit_asset': 'Near.USDC'}
    result = splitter.split(completed)
    assert result['split_applied']
    assert int(result['owner_units']) + int(result['operator_units']) == 1_000_000

    zero = {'phase': 'COMPLETED', 'paper_profit_units': '0'}
    assert not splitter.split(zero)['split_applied']


def test_mandate_diff_alignment():
    diff = MandateDiff()
    paper = [{'guard': 'slippage', 'ok': True}, {'guard': 'inventory', 'ok': False}]
    live = [{'guard': 'slippage', 'ok': True}, {'guard': 'inventory', 'ok': False}]
    result = diff.compare(paper, live)
    assert result['aligned']
    assert 'inventory' in result['paper_failed']


def test_liquidity_listing_shape():
    with tempfile.TemporaryDirectory() as d:
        store = TradingStore(Path(d) / 't.sqlite')
        listing = LiquidityListing(MandateProof(store))
        out = listing.listing([], 0)
        assert out['schema'] == 'x3.liquidity.listing.v1'
        assert isinstance(out['entries'], list)
