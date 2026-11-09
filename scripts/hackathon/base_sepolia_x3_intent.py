#!/usr/bin/env python3
"""X3Lang Python frontend for ONE explicitly authorized Base Sepolia testnet intent.

No mainnet route; this does not pretend to compile Rust VM bytecode. The source
is parsed by the repository's actual X3Lang frontend and typechecked against
its registry, then lowered only to an immutable, testnet-specific policy record.
"""
from __future__ import annotations
import hashlib
import re
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0,str(ROOT/'x3-lang'))
import cli
import registry
import typechecker

SOURCE = ROOT/'x3-lang/examples/base_sepolia_to_near_testnet_1usdc.x3'
ORIGIN = '0x5aF0c1BF95B3A158f6Fc48e3DED35903b2fa0721'
RECEIVER = 'x3lang.testnet'
NONCE = 'x3_base_testnet_trade_001'

class ProgramRefused(ValueError):pass

def require(cond: bool,message: str) -> None:
    if not cond:raise ProgramRefused(message)

def compile_testnet_intent(path: Path = SOURCE, *, expected_nonce: str = NONCE) -> dict:
    require(isinstance(expected_nonce, str) and re.fullmatch(r'x3_[a-z0-9_]{5,61}', expected_nonce) is not None, 'INVALID_TESTNET_OWNER_NONCE')
    source = path.read_bytes()
    try:
        result=cli.parse_source(source.decode("utf-8"))
    except cli.X3ParseError as exc:
        raise ProgramRefused("X3LANG_PARSE_FAILED: " + str(exc)) from exc
    valid,errors=typechecker.typecheck(result)
    require(valid,'X3LANG_TYPECHECK_FAILED: '+str([str(e) for e in errors]))
    fr=result.get('from') or {};to=result.get('to') or {}
    require(registry.normalize_chain(fr.get('chain'))=='basesepolia','TESTNET_SOURCE_CHAIN_MISMATCH')
    require(fr.get('asset')=='USDC' and str(fr.get('amount'))=='1000000','TESTNET_SOURCE_ASSET_AMOUNT_MISMATCH')
    require(str(fr.get('receiver')).lower()==ORIGIN.lower(),'TESTNET_SOURCE_WALLET_MISMATCH')
    require(registry.normalize_chain(to.get('chain'))=='near' and to.get('asset')=='USDC' and to.get('receiver')==RECEIVER,'TESTNET_DESTINATION_MISMATCH')
    route=result.get('route') or []
    require(len(route)==1 and route[0].get('type')=='bridge' and str(route[0].get('via')).lower() in registry.NEAR_INTENTS_VIAS,'TESTNET_ROUTE_UNSUPPORTED')
    leg=route[0]
    require(registry.normalize_chain((leg.get('from_ref') or {}).get('chain'))=='basesepolia' and (leg.get('from_ref') or {}).get('asset')=='USDC','TESTNET_ROUTE_SOURCE_MISMATCH')
    require(registry.normalize_chain((leg.get('to_ref') or {}).get('chain'))=='near' and (leg.get('to_ref') or {}).get('asset')=='USDC','TESTNET_ROUTE_DEST_MISMATCH')
    require(str(leg.get('min_receive'))=='980000','TESTNET_MINIMUM_MISMATCH')
    requires=result.get('requires') or []
    require(len(requires)==2 and any(r.get('kind')=='slippage' and r.get('op')=='<=' and r.get('value')=='1%' for r in requires),'TESTNET_SLIPPAGE_MISMATCH')
    require(any(r.get('kind')=='nonce' and r.get('value')=='unused '+expected_nonce for r in requires),'TESTNET_NONCE_MISMATCH')
    policy=result.get('policies') or {}
    timeout=policy.get('timeout') or {}; action=timeout.get('action') or {}
    require(timeout.get('duration')=='30m' and action.get('type')=='refund' and registry.normalize_chain(action.get('chain'))=='basesepolia' and action.get('asset')=='USDC' and str(action.get('to')).lower()==ORIGIN.lower(),'TESTNET_REFUND_MISMATCH')
    require((policy.get('on_fail') or {}).get('type')=='rollback','TESTNET_ROLLBACK_MISMATCH')
    return {'classification':'x3lang_python_frontend_testnet_policy_not_vm_bytecode',
            'program_sha256':hashlib.sha256(source).hexdigest(), 'source_path':str(path),
            'origin_chain':'basesepolia','source_account':ORIGIN,'origin_asset':'USDC',
            'origin_amount':1000000,'destination_chain':'near','destination_account':RECEIVER,
            'destination_asset':'USDC','minimum_out':980000,'slippage_max_bps':100,
            'nonce':expected_nonce,'refund_to':ORIGIN,'timeout_minutes':30}

if __name__=='__main__':
    import json
    print(json.dumps(compile_testnet_intent(),indent=2))
