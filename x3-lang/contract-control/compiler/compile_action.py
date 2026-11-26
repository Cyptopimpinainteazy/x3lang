"""Offline bounded X3CCP EVM source profile; no signing or RPC."""
import hashlib
import re
import subprocess

class PolicyError(ValueError):
    pass


def require(ok, message):
    if not ok:
        raise PolicyError(message)


def uint(value):
    require(type(value) is int and 0 <= value < 2**256, 'invalid uint256')
    return value.to_bytes(32, 'big')


def decimal(value):
    require(re.fullmatch(r'0|[1-9][0-9]{0,77}', value) is not None, 'invalid decimal')
    n = int(value)
    uint(n)
    return n


def address(value):
    require(re.fullmatch(r'0x[0-9a-f]{40}', value) is not None, 'invalid lowercase address')
    raw = bytes.fromhex(value[2:])
    require(any(raw), 'zero address')
    return bytes(12) + raw


def keccak(data):
    # OpenSSL 3 with KECCAK-256 support is required; never fall back to SHA3.
    try:
        result = subprocess.run(['openssl', 'dgst', '-keccak-256', '-binary'],
                                input=data, capture_output=True, check=True, timeout=5)
    except (OSError, subprocess.SubprocessError) as exc:
        raise PolicyError('Keccak-256 unavailable') from exc
    require(len(result.stdout) == 32, 'invalid Keccak digest')
    return result.stdout


def compile_action(source, *, value, nonce, policy_version, now):
    require(type(source) is bytes and 0 < len(source) <= 8192, 'invalid source size')
    try:
        lines = source.decode('ascii').split('\n')
    except UnicodeError as exc:
        raise PolicyError('source must be ASCII') from exc
    lines = [s.strip(' \t\r') for s in lines if s.strip(' \t\r')]
    require(len(lines) == 9 and re.fullmatch(r'contract_policy [a-z][a-z0-9_]{0,63} \{', lines[0]) is not None and lines[-1] == '}', 'unsupported policy syntax')
    keys = ('chain_id', 'executor', 'target', 'agent', 'calldata', 'max_value', 'expiry')
    terms = {}
    for line in lines[1:-1]:
        parts = line.split()
        require(len(parts) == 2 and parts[0] in keys and parts[0] not in terms, 'unknown or duplicate term')
        terms[parts[0]] = parts[1]
    require(set(terms) == set(keys), 'missing term')
    chain = decimal(terms['chain_id'])
    require(chain == 84532, 'Base Sepolia only')
    executor, target, agent = (address(terms[k]) for k in ('executor','target','agent'))
    require(executor != target, 'executor cannot be target')
    raw = terms['calldata']
    require(re.fullmatch(r'0x[0-9a-f]{8}(?:[0-9a-f]{64}){0,16}', raw) is not None, 'only selector plus up to 16 ABI words')
    data = bytes.fromhex(raw[2:])
    cap, expiry = decimal(terms['max_value']), decimal(terms['expiry'])
    for n in (value, nonce, policy_version, now):
        uint(n)
    require(value <= cap and policy_version > 0 and now < expiry, 'value, version or expiry denied')
    policy_hash = hashlib.sha256(source).digest()
    data_hash = keccak(data)
    encoded = b''.join((keccak(b'X3ContractActionV1'), uint(chain), executor, target,
                       data[:4] + bytes(28), data_hash, uint(value), agent, policy_hash,
                       uint(policy_version), uint(nonce), uint(expiry)))
    hx = lambda b: '0x' + b.hex()
    action = dict(chainId=str(chain), executor=terms['executor'], target=terms['target'],
                  selector=hx(data[:4]), calldataHash=hx(data_hash), value=str(value),
                  agent=terms['agent'], policyHash=hx(policy_hash), policyVersion=str(policy_version),
                  nonce=str(nonce), expiry=str(expiry))
    return dict(schema='x3.contract-action.v1', action=action, calldata=raw,
                action_id=hx(keccak(encoded)), owner_authorized=False, deployment_verified=False)
