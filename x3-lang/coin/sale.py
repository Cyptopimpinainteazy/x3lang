"""Offline equal-share quotes against supplied treasury inventory, in base units."""
import re

from . import CoinError, U64_MAX, _fields, _uint


def network_id(value):
    if not isinstance(value, str) or not re.fullmatch(r"[a-z0-9][a-z0-9._:-]{0,63}", value):
        raise CoinError("network id requires a lowercase ASCII identifier of 1..64 characters")
    return value


def plan_sale(spec):
    """Quote one delivery and the economic share assigned to each treasury.

    Each available amount is that treasury's uncommitted sale quota. The float
    is a separate view of spendable destination inventory and is never added to
    the quotas. Remote payables and destination receivables refer to the same
    units. The inputs are caller-supplied and this function has no side effects.
    """
    _fields(spec, ("decimals", "networks", "destination", "amount", "destination_float"), "sale")
    decimals = spec["decimals"]
    if type(decimals) is not int or not 0 <= decimals <= 18:
        raise CoinError("decimals must be an integer from 0 to 18")
    networks = spec["networks"]
    if not isinstance(networks, list) or not networks:
        raise CoinError("networks must be a nonempty list of participating treasury quotas")
    available = {}
    for entry in networks:
        _fields(entry, ("id", "available"), "network")
        network = network_id(entry["id"])
        if network in available:
            raise CoinError(f"duplicate network id: {network}")
        available[network] = _uint(entry["available"], f"{network}.available", U64_MAX)
    destination = spec["destination"]
    if not isinstance(destination, str) or destination not in available:
        raise CoinError("destination must be one of the participating networks")
    amount = _uint(spec["amount"], "amount", U64_MAX, 1)
    balance = _uint(spec["destination_float"], "destination_float", U64_MAX)
    charge, remainder = divmod(amount, len(available))
    if remainder:
        raise CoinError("amount cannot be split equally across the networks in whole base units")
    shares = []
    for network in sorted(available):
        if available[network] < charge:
            raise CoinError(f"{network} has insufficient uncommitted quota for its share")
        shares.append({
            "network": network,
            "charge": str(charge),
            "remote_payable": str(charge if network != destination else 0),
        })
    if balance < amount:
        raise CoinError("insufficient destination float for the full local delivery")
    return {
        "status": "quote_only",
        "decimals": decimals,
        "destination": destination,
        "amount": str(amount),
        "shares": shares,
        "destination_local_share": str(charge),
        "destination_receivable": str(amount - charge),
        "destination_float_after_delivery": str(balance - amount),
    }
