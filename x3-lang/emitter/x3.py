import json
from typing import Any, Dict


class ProofRequiredError(Exception):
    code = 'X3_PROOF_REQUIRED'


def _serialize_x3_payload(payload: Dict[str, Any]) -> str:
    return '0x' + json.dumps(payload, sort_keys=True).encode('utf-8').hex()


def verify_proof_bundle(payload: Dict[str, Any], proof_bundle: Dict[str, Any]) -> bool:
    """Verifier contract boundary.

    Production execution must supply a backend-produced proof bundle containing
    the payload hash, finality checkpoint, settlement root, and verifier status.
    The emitter never manufactures those values.
    """
    if not isinstance(proof_bundle, dict):
        return False
    required = {'payload_hash', 'checkpoint', 'settlement_root', 'verifier', 'finality'}
    if not required.issubset(proof_bundle):
        return False
    return proof_bundle.get('verifier', {}).get('status') == 'verified' and bool(proof_bundle.get('finality', {}).get('confirmed'))


def emit(plan: Dict[str, Any], step: Dict[str, Any], *, proof_bundle: Dict[str, Any] | None = None, dry_run: bool = False) -> Dict[str, Any]:
    amount = step.get('amount') or plan.get('from', {}).get('amount') or 0
    try:
        amount = int(float(amount))
    except Exception:
        amount = 0

    payload = {
        'action': 'settle',
        'asset': step.get('asset'),
        'amount': amount,
        'source_chain': step.get('from_ref', {}).get('chain') or step.get('chain'),
        'target_chain': step.get('to_ref', {}).get('chain') or plan.get('to', {}).get('chain'),
        'receiver': step.get('receiver') or plan.get('to', {}).get('receiver'),
        'extras': {'intent': plan.get('intent'), 'path_step': step.get('step')},
    }
    tx = {
        'type': 'bridge',
        'chain': 'x3',
        'via': step.get('via'),
        'asset': step.get('asset'),
        'amount': amount,
        'payload': payload,
        'transaction_id': f"x3-bridge-{plan.get('intent')}-{step.get('step')}",
        'transaction_bytes': _serialize_x3_payload(payload),
        'proof_required': True,
        'dry_run': dry_run,
    }
    if dry_run:
        raise ProofRequiredError('dry-run cannot produce a production proof bundle')

    if not verify_proof_bundle(payload, proof_bundle or {}):
        raise ProofRequiredError('missing or invalid verifier proof/finality/settlement bundle')

    tx['proof_bundle'] = proof_bundle
    return tx
