"""X3-PIR v1: offline provenance bindings, never execution authorization."""
from dataclasses import dataclass
from enum import Enum
import hashlib
import hmac
import re


class Profile(Enum):
    EVM_ACTION = "evm.action.v1"
    AGENT_ADMISSION = "agent.admission.v1"
    NEAR_BSC_WINDOW = "near-bsc.window.v1"


class PolicyIRError(ValueError):
    pass


DOMAIN = b"X3/PIR/BINDING/1\x00"
MAX_SOURCE_BYTES = 65536
_SCOPE = re.compile(r"[a-zA-Z0-9][a-zA-Z0-9:._/-]{0,127}", re.ASCII)


@dataclass(frozen=True)
class Record:
    encoded: bytes
    digest: str


def bind(profile: Profile, source: bytes, scope: str, native_id: bytes) -> Record:
    """Bind independently selected source, scope and native identity exactly.

    Does not validate native authorization, source semantics or network state.
    A native identifier is an opaque, profile-specific 32-byte value.
    """
    if type(profile) is not Profile:
        raise PolicyIRError("unknown profile")
    if type(source) is not bytes or not 0 < len(source) <= MAX_SOURCE_BYTES:
        raise PolicyIRError("invalid source size")
    if type(scope) is not str or _SCOPE.fullmatch(scope) is None:
        raise PolicyIRError("invalid scope")
    if type(native_id) is not bytes or len(native_id) != 32:
        raise PolicyIRError("invalid native identity")
    p, s = profile.value.encode("ascii"), scope.encode("ascii")
    encoded = (DOMAIN + len(p).to_bytes(1, "big") + p
               + len(s).to_bytes(1, "big") + s
               + hashlib.sha256(source).digest() + native_id)
    return Record(encoded, hashlib.sha256(encoded).hexdigest())


def verify(encoded: bytes, profile: Profile, source: bytes, scope: str, native_id: bytes) -> bool:
    """Compare to a binding rebuilt using independently trusted inputs."""
    if type(encoded) is not bytes:
        return False
    try:
        return hmac.compare_digest(encoded, bind(profile, source, scope, native_id).encoded)
    except PolicyIRError:
        return False


def bind_agent_admission(source: bytes, scope: str, child, parent, *, nonce: str, expires_at: int) -> Record:
    """Use the existing signed admission payload; no signature verification here."""
    from agent_admission_signed import approval_payload
    native = hashlib.sha256(approval_payload(child, parent, nonce=nonce, expires_at=expires_at)).digest()
    return bind(Profile.AGENT_ADMISSION, source, scope, native)


def bind_settlement_window(source: bytes, scope: str, **terms) -> Record:
    """Use the original settlement encoder; no chain observation/finality proof."""
    from bridge.settlement import window_id
    return bind(Profile.NEAR_BSC_WINDOW, source, scope, window_id(**terms))
