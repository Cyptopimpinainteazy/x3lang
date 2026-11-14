"""Cloudflare Access JWT check at the Forge origin.

Same rules as `deploy/x3star/server.mjs`: RS256 only, key from the team's
`/cdn-cgi/access/certs`, issuer = team domain, audience = the Access app AUD, not expired, and
the email must be the owner's. The edge already enforces Access; this makes the origin refuse
anything that reaches it some other way.
"""
from __future__ import annotations

import base64
import json
import re
import threading
import time
import urllib.request
from typing import Any, Callable, Dict, Optional, Tuple

from cryptography.exceptions import InvalidSignature
from cryptography.hazmat.primitives import hashes
from cryptography.hazmat.primitives.asymmetric import padding, rsa

JWKS_TTL_S = 3600
JWKS_MIN_REFRESH_S = 30
CLOCK_SKEW_S = 60
TEAM_RE = re.compile(r"https://[a-z0-9-]+\.cloudflareaccess\.com")
AUD_RE = re.compile(r"[0-9a-f]{32,128}", re.I)
EMAIL_RE = re.compile(r"[^\s@]+@[^\s@]+\.[^\s@]+")
PART_RE = re.compile(r"[A-Za-z0-9_-]+")

Fetch = Callable[[str], Dict[str, Any]]


def _b64(part: str) -> bytes:
    return base64.urlsafe_b64decode(part + "=" * (-len(part) % 4))


def _fetch_json(url: str) -> Dict[str, Any]:
    req = urllib.request.Request(url, headers={"Accept": "application/json", "User-Agent": "x3-agent-forge/1"})
    with urllib.request.urlopen(req, timeout=10) as resp:
        return json.loads(resp.read())


class AccessVerifier:
    def __init__(self, team_domain: str, aud: str, owner_email: str, *, fetch: Fetch = _fetch_json,
                 clock: Callable[[], float] = time.time):
        if not TEAM_RE.fullmatch(team_domain or ""):
            raise ValueError("access team domain must look like https://<team>.cloudflareaccess.com")
        if not AUD_RE.fullmatch(aud or ""):
            raise ValueError("access AUD must be the Access application's AUD tag (hex)")
        email = (owner_email or "").strip().lower()
        if not EMAIL_RE.fullmatch(email):
            raise ValueError("owner email is required in access mode")
        self.team, self.aud, self.owner = team_domain, aud, email
        self.fetch, self.clock = fetch, clock
        self.keys: Dict[str, rsa.RSAPublicKey] = {}
        self.fetched_at = 0.0
        self.lock = threading.Lock()

    def _refresh(self) -> None:
        body = self.fetch(f"{self.team}/cdn-cgi/access/certs")
        keys: Dict[str, rsa.RSAPublicKey] = {}
        for jwk in body.get("keys", []) if isinstance(body, dict) else []:
            if not isinstance(jwk, dict) or jwk.get("kty") != "RSA" or not isinstance(jwk.get("kid"), str):
                continue
            n = int.from_bytes(_b64(jwk["n"]), "big")
            e = int.from_bytes(_b64(jwk["e"]), "big")
            keys[jwk["kid"]] = rsa.RSAPublicNumbers(e, n).public_key()
        if not keys:
            raise ValueError("certs response has no RSA keys")
        self.keys, self.fetched_at = keys, self.clock()

    def _key(self, kid: str) -> Optional[rsa.RSAPublicKey]:
        with self.lock:
            age = self.clock() - self.fetched_at
            if age > JWKS_TTL_S or (kid not in self.keys and age > JWKS_MIN_REFRESH_S):
                self._refresh()
            return self.keys.get(kid)

    def verify(self, token: Optional[str]) -> Tuple[bool, str]:
        if not isinstance(token, str) or len(token) > 16384:
            return False, "missing token"
        parts = token.split(".")
        if len(parts) != 3 or not all(PART_RE.fullmatch(p) for p in parts):
            return False, "malformed token"
        try:
            header, payload = json.loads(_b64(parts[0])), json.loads(_b64(parts[1]))
        except (ValueError, UnicodeDecodeError):
            return False, "malformed token"
        if not isinstance(header, dict) or not isinstance(payload, dict):
            return False, "malformed token"
        if header.get("alg") != "RS256":
            return False, "unsupported algorithm"
        if not isinstance(header.get("kid"), str):
            return False, "missing key id"
        try:
            key = self._key(header["kid"])
        except Exception as exc:  # network or parse failure: refuse
            return False, f"cannot load signing keys: {type(exc).__name__}"
        if key is None:
            return False, "unknown key id"
        try:
            key.verify(_b64(parts[2]), f"{parts[0]}.{parts[1]}".encode(), padding.PKCS1v15(), hashes.SHA256())
        except (InvalidSignature, ValueError):
            return False, "bad signature"
        now = self.clock()
        if payload.get("iss") != self.team:
            return False, "wrong issuer"
        auds = payload.get("aud") if isinstance(payload.get("aud"), list) else [payload.get("aud")]
        if self.aud not in auds:
            return False, "wrong audience"
        exp, nbf = payload.get("exp"), payload.get("nbf")
        if type(exp) not in (int, float) or exp + CLOCK_SKEW_S <= now:
            return False, "expired"
        if type(nbf) in (int, float) and nbf - CLOCK_SKEW_S > now:
            return False, "not yet valid"
        email = payload.get("email")
        if not isinstance(email, str) or email.lower() != self.owner:
            return False, "wrong owner"
        return True, self.owner
