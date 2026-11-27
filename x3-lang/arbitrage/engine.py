"""Integer-only closed-cycle projections. A projected floor is NOT a reserved fill."""
from dataclasses import asdict, dataclass
from hashlib import sha256
from typing import Optional, Tuple

from near_liquidity.quote import AssetRef, canonical_json, parse_uint


def uint(value, name, *, positive=False):
    value = parse_uint(value, name)
    if positive and value == 0:
        raise ValueError(f'{name} must be positive')
    return value


@dataclass(frozen=True)
class Leg:
    provider: str
    asset_in: AssetRef
    asset_out: AssetRef
    amount_in: int
    expected_out: int
    minimum_out: int
    observed_at: int
    expires_at: int
    evidence: str = 'constructed_demo'
    reference: str = ''

    def __post_init__(self):
        if self.provider not in {'oneclick', 'coinbase', 'kraken'}:
            raise ValueError('unsupported provider')
        if not isinstance(self.asset_in, AssetRef) or not isinstance(self.asset_out, AssetRef):
            raise ValueError('chain-qualified assets required')
        for name in ('amount_in', 'expected_out', 'minimum_out', 'observed_at', 'expires_at'):
            if type(getattr(self, name)) is not int:
                raise ValueError(f'{name} must be an integer')
            uint(getattr(self, name), name, positive=True)
        if self.minimum_out > self.expected_out or self.expires_at <= self.observed_at:
            raise ValueError('invalid output floor or lifetime')
        if self.evidence not in {'constructed_demo', 'provider_dry_quote', 'public_orderbook'}:
            raise ValueError('unknown evidence level')
        if len(self.reference) > 256:
            raise ValueError('reference too long')


@dataclass(frozen=True)
class Route:
    label: str
    kind: str
    legs: Tuple[Leg, ...]
    external_cost_units: Optional[int]
    cost_source: str
    atomic: bool = False

    def __post_init__(self):
        if not isinstance(self.label, str) or not 1 <= len(self.label) <= 160:
            raise ValueError('route label required (max 160 characters)')
        if self.kind not in {'cross-chain', 'cex-inventory'}:
            raise ValueError('unsupported route kind')
        if not isinstance(self.legs, tuple) or not 2 <= len(self.legs) <= 4:
            raise ValueError('route requires 2..4 legs')
        if not all(isinstance(leg, Leg) for leg in self.legs):
            raise ValueError('all route legs must be Leg records')
        if self.external_cost_units is not None:
            if type(self.external_cost_units) is not int:
                raise ValueError('external costs must be integer base units')
            uint(self.external_cost_units, 'external_cost_units')
        if not isinstance(self.cost_source, str) or not 1 <= len(self.cost_source) <= 300:
            raise ValueError('cost provenance required')
        if self.atomic is not False:
            raise ValueError('these adapters cannot establish cross-chain atomicity')

    @property
    def id(self):
        return sha256(canonical_json(asdict(self))).hexdigest()

    @property
    def demo(self):
        return any(leg.evidence == 'constructed_demo' for leg in self.legs)


def assess(route, *, now, max_age_s=30, min_profit_bps=0, min_remaining_s=10):
    for value, name in ((now, 'now'), (max_age_s, 'max_age_s'),
                        (min_profit_bps, 'min_profit_bps'), (min_remaining_s, 'min_remaining_s')):
        if type(value) is not int:
            raise ValueError(f'{name} must be an integer')
        uint(value, name)
    reasons = []
    legs = route.legs
    if legs[0].asset_in != legs[-1].asset_out:
        reasons.append('NOT_CLOSED_CYCLE')
    for index, leg in enumerate(legs):
        if now < leg.observed_at or now - leg.observed_at > max_age_s:
            reasons.append('STALE_QUOTE')
        if leg.expires_at - now < min_remaining_s or leg.expires_at <= now:
            reasons.append('EXPIRED_OR_EXPIRING')
        if index:
            previous = legs[index - 1]
            if previous.asset_out != leg.asset_in:
                reasons.append('ASSET_OR_CHAIN_MISMATCH')
            if previous.minimum_out < leg.amount_in:
                reasons.append('INSUFFICIENT_INTERMEDIATE_OUTPUT')
            # No unvalued residual inventory in reported round-trip P&L.
            if previous.minimum_out > leg.amount_in:
                reasons.append('UNVALUED_INTERMEDIATE_REMAINDER')
    if route.external_cost_units is None:
        reasons.append('UNKNOWN_EXTERNAL_COSTS')
    if route.kind == 'cex-inventory':
        reasons.append('PREFUNDED_INVENTORY_AND_REBALANCE_REQUIRED')
    initial = legs[0].amount_in
    closed = legs[0].asset_in == legs[-1].asset_out
    net = legs[-1].minimum_out - initial - route.external_cost_units if closed and route.external_cost_units is not None else None
    bps = net * 10000 // initial if net is not None else None
    if bps is not None and (net <= 0 or bps < min_profit_bps):
        reasons.append('BELOW_PROFIT_THRESHOLD')
    return {
        'id': route.id, 'label': route.label, 'kind': route.kind,
        'demo': route.demo, 'legs': [leg_json(leg) for leg in legs],
        'input_units': str(initial), 'numeraire': legs[0].asset_in.to_json(),
        'external_cost_units': None if route.external_cost_units is None else str(route.external_cost_units),
        'cost_source': route.cost_source,
        'gross_projected_profit_units': str(legs[-1].minimum_out - initial) if closed else None,
        'net_profit_units': None if net is None else str(net), 'net_profit_bps': bps,
        'paper_eligible': not reasons, 'reasons': sorted(set(reasons)),
        'eligible_for_wallet_authorization': False, 'atomic': False,
        'risk': 'Sequential quotes do not reserve liquidity. Price, bridge and partial-execution risk remain.',
        'assessed_at': now, 'expires_at': min(leg.expires_at for leg in legs),
    }


def leg_json(leg):
    result = asdict(leg)
    for name in ('amount_in', 'expected_out', 'minimum_out'):
        result[name] = str(result[name])
    return result


def rank(routes, *, now, min_profit_bps=0):
    rows = [assess(route, now=now, min_profit_bps=min_profit_bps) for route in routes]
    return sorted(rows, key=lambda row: (not row['paper_eligible'], row['net_profit_bps'] is None,
                                        -(row['net_profit_bps'] or 0), row['id']))
