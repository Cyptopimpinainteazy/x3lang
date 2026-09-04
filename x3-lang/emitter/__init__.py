import importlib.util
import os


def _load_emitter(name):
    base = os.path.dirname(__file__)
    path = os.path.join(base, f'{name}.py')
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


_emit_evm = _load_emitter('evm')
_emit_svm = _load_emitter('svm')
_emit_x3 = _load_emitter('x3')


def emit(plan, *, dry_run=False, proof_bundle=None):
    emitted = []
    for step in plan.get('steps', []):
        action = step['action']
        if action == 'swap':
            dex = step.get('dex', '').lower()
            if dex == 'raydium':
                emitted.append(_emit_svm.emit(plan, step))
            elif dex == 'uniswap':
                emitted.append(_emit_evm.emit(plan, step))
            else:
                emitted.append({'type': 'swap', 'chain': 'unknown', 'error': f'unsupported dex {dex}'})
        elif action == 'bridge':
            emitted.append(_emit_x3.emit(plan, step, dry_run=dry_run, proof_bundle=proof_bundle))
        elif action == 'lock':
            emitted.append(_emit_evm.emit(plan, step))
        elif action == 'release':
            emitted.append(_emit_evm.emit(plan, step))
        elif action == 'mint':
            emitted.append(_emit_svm.emit(plan, step))
        elif action == 'burn':
            emitted.append(_emit_svm.emit(plan, step))
        else:
            emitted.append({'type': 'unknown', 'step': step})
    return {'emitted': emitted}
