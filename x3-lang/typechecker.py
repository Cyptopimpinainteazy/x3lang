"""Structured validator for X3 intent JSON."""
from dataclasses import dataclass
from typing import Any, Dict, List, Tuple
import importlib.util
import os


def load_registry():
    registry_path = os.path.join(os.path.dirname(__file__), 'registry.py')
    spec = importlib.util.spec_from_file_location('registry', registry_path)
    registry = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(registry)
    return registry

registry = load_registry()
ALLOWED_CHAINS = set(registry.CHAIN_DOMAINS.keys())

@dataclass
class X3ValidationError:
    code: str
    message: str
    field: str
    value: Any = None
    def to_dict(self):
        out = {"code": self.code, "message": self.message, "field": self.field}
        if self.value is not None:
            out["value"] = self.value
        return out
    def __str__(self):
        return f"{self.code}: {self.field}: {self.message}"


def _err(errors, code, message, field, value=None):
    errors.append(X3ValidationError(code, message, field, value))


def _norm_chain(errors, chain, field):
    norm = registry.normalize_chain(chain)
    if norm not in ALLOWED_CHAINS:
        _err(errors, 'X3_INVALID_CHAIN', f'unsupported chain {chain!r}', field, chain)
        return None
    return norm


def _check_asset(errors, chain, asset, field):
    norm = _norm_chain(errors, chain, field + '.chain')
    if norm and asset not in registry.ASSETS_BY_CHAIN.get(norm, set()):
        _err(errors, 'X3_INVALID_ASSET', f'asset {asset!r} not registered on {norm}', field + '.asset', asset)
    return norm


def _check_receiver(errors, chain, receiver, field):
    if not registry.is_valid_receiver(chain, receiver):
        _err(errors, 'X3_INVALID_RECEIVER', f'invalid receiver for {chain}', field, receiver)


def typecheck(intent: dict) -> Tuple[bool, List[Any]]:
    errors: List[X3ValidationError] = []
    if not isinstance(intent.get('intent'), str) or not intent.get('intent'):
        _err(errors, 'X3_MISSING_INTENT', 'missing or invalid intent name', 'intent')

    for endpoint in ('from', 'to'):
        val = intent.get(endpoint)
        if not isinstance(val, dict):
            _err(errors, 'X3_MISSING_ENDPOINT', f'missing {endpoint} endpoint', endpoint)
            continue
        chain = _check_asset(errors, val.get('chain'), val.get('asset'), endpoint)
        _check_receiver(errors, chain, val.get('receiver'), endpoint + '.receiver')
        if endpoint == 'from' and val.get('amount') is not None:
            try:
                if float(str(val.get('amount'))) <= 0:
                    raise ValueError()
            except Exception:
                _err(errors, 'X3_INVALID_AMOUNT', 'amount must be positive numeric', endpoint + '.amount', val.get('amount'))

    route = intent.get('route') or intent.get('path')
    if not isinstance(route, list) or not route:
        _err(errors, 'X3_EMPTY_ROUTE', 'route/path must contain at least one operation', 'route')
    else:
        seen_nonce = False
        for idx, step in enumerate(route):
            field = f'route[{idx}]'
            if not isinstance(step, dict):
                _err(errors, 'X3_INVALID_OPERATION', 'operation must be an object', field)
                continue
            typ = step.get('type')
            if typ not in registry.SUPPORTED_OPERATIONS:
                _err(errors, 'X3_INVALID_OPERATION', f'unsupported operation {typ!r}', field + '.type', typ)
                continue
            if typ == 'swap':
                if str(step.get('dex', '')).lower() not in registry.KNOWN_DEXS:
                    _err(errors, 'X3_INVALID_DEX', 'unknown DEX', field + '.dex', step.get('dex'))
                fr = step.get('from_ref') or {'chain': intent.get('from', {}).get('chain'), 'asset': step.get('from')}
                to = step.get('to_ref') or {'chain': fr.get('chain'), 'asset': step.get('to')}
                _check_asset(errors, fr.get('chain'), fr.get('asset'), field + '.from')
                _check_asset(errors, to.get('chain'), to.get('asset'), field + '.to')
            elif typ == 'bridge':
                if str(step.get('via', '')).lower() not in registry.KNOWN_BRIDGES:
                    _err(errors, 'X3_INVALID_BRIDGE', 'unknown bridge adapter', field + '.via', step.get('via'))
                fr = step.get('from_ref') or {'chain': intent.get('from', {}).get('chain'), 'asset': step.get('asset')}
                to = step.get('to_ref') or {'chain': intent.get('to', {}).get('chain'), 'asset': step.get('to_asset') or step.get('asset')}
                _check_asset(errors, fr.get('chain'), fr.get('asset'), field + '.from')
                _check_asset(errors, to.get('chain'), to.get('asset'), field + '.to')
                _check_receiver(errors, to.get('chain'), step.get('receiver') or intent.get('to', {}).get('receiver'), field + '.receiver')
            else:
                _check_asset(errors, step.get('chain'), step.get('asset'), field)

    requires = intent.get('requires', [])
    if requires and not isinstance(requires, list):
        _err(errors, 'X3_INVALID_REQUIRES', 'requires must be an array', 'requires')
    for idx, req in enumerate(requires if isinstance(requires, list) else []):
        kind = req.get('kind') if isinstance(req, dict) else None
        if kind not in registry.REQUIRE_KINDS:
            _err(errors, 'X3_INVALID_REQUIRE', 'unknown require kind', f'requires[{idx}].kind', kind)
        if kind == 'finality':
            _norm_chain(errors, req.get('chain'), f'requires[{idx}].chain')
            try:
                if int(req.get('value')) <= 0: raise ValueError()
            except Exception:
                _err(errors, 'X3_INVALID_FINALITY', 'finality confirmations must be positive integer', f'requires[{idx}].value', req.get('value'))
        if kind == 'nonce':
            seen_nonce = True

    policies = intent.get('policies', {})
    for key in ('timeout', 'on_fail'):
        action = policies.get(key, {}).get('action') if key == 'timeout' else policies.get(key)
        if isinstance(action, dict) and action.get('type') == 'refund':
            _check_asset(errors, action.get('chain'), action.get('asset'), f'policies.{key}.asset')
            _check_receiver(errors, action.get('chain'), action.get('to'), f'policies.{key}.to')

    return (len(errors) == 0, errors)


def errors_to_json(errors):
    return [e.to_dict() if hasattr(e, 'to_dict') else {"code": "X3_VALIDATION_ERROR", "message": str(e)} for e in errors]

if __name__ == '__main__':
    import json, sys
    ok, errs = typecheck(json.load(sys.stdin))
    print(json.dumps({"status": "ok" if ok else "error", "errors": errors_to_json(errs)}, indent=2))
    sys.exit(0 if ok else 1)
