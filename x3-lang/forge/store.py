"""The Forge host: High Command key, births into the Agent Law runtime, roster, revocation,
the agent mesh, and manifest export/import.

Authority lives in two places, and neither is this file:
- the Agent Law runtime (`agent_runtime.Runtime`), which records a birth only with the
  operator's Ed25519 signature over its own `genesis_payload`, and
- x3-guard on NEAR, which holds the mandate only after the owner sends `create_mandate`.
The Forge never holds the operator key, the agent key or a NEAR key. It prints the
`create_mandate` / `revoke_mandate` calls for the owner to sign and send.
"""
from __future__ import annotations

import json
import secrets
import tempfile
import threading
import time
from pathlib import Path
from typing import Any, Callable, Dict, List, Optional

from cryptography.exceptions import InvalidSignature
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey

import near_intents as ni
from agent_runtime import Runtime

from . import core, proving
from .connections import ConnectionStore, Transport, urllib_transport

CLAIM_DOMAIN = "X3/FORGE/CLAIM/1"
REVOKE_DOMAIN = "X3/FORGE/REVOKE/1"
REVOKE_CONN_DOMAIN = "X3/FORGE/REVOKE_CONNECTION/1"
MESH_DOMAIN = "X3/FORGE/MESH/1"
BIRTH_TTL_S = 600
MESH_TTL_S = 300
MAX_BODY = 2048


def _hex(value: Any, length: int, what: str) -> bytes:
    if not isinstance(value, str) or len(value) != 2 * length:
        raise core.ForgeError("X3_FORGE_KEY", f"{what} must be {length} bytes of hex", "keys")
    try:
        return bytes.fromhex(value)
    except ValueError as exc:
        raise core.ForgeError("X3_FORGE_KEY", f"{what} is not hex", "keys") from exc


def verify_sig(public: bytes, signature: bytes, payload: bytes, what: str) -> None:
    try:
        Ed25519PublicKey.from_public_bytes(public).verify(signature, payload)
    except (InvalidSignature, ValueError) as exc:
        raise core.ForgeError("X3_FORGE_SIGNATURE", f"{what}: signature does not verify", "keys") from exc


class Forge:
    def __init__(self, data_dir: Path, reg: core.Registry, assets: core.Assets, *,
                 transport: Transport = urllib_transport, clock: Callable[[], float] = time.time,
                 setup_token: Optional[str] = None):
        self.dir = Path(data_dir)
        self.dir.mkdir(parents=True, exist_ok=True)
        (self.dir / "agents").mkdir(exist_ok=True)
        self.reg, self.assets, self.clock = reg, assets, clock
        self.connections = ConnectionStore(reg, self.dir, transport=transport, clock=clock)
        self.lock = threading.RLock()
        self.pending: Dict[str, Dict[str, Any]] = {}
        self.mesh_rate: Dict[str, List[float]] = {}
        self.setup_token = setup_token or secrets.token_urlsafe(18)
        self.runtime: Optional[Runtime] = None
        op = self.dir / "operator.json"
        if op.exists():
            self._open_runtime(bytes.fromhex(json.loads(op.read_text())["public_key"]))

    # -- High Command ----------------------------------------------------------------------
    def _open_runtime(self, public_key: bytes) -> None:
        self.operator_key = public_key
        self.runtime = Runtime(self.dir / "runtime.sqlite3", public_key, {}, clock=self.clock)

    def status(self) -> Dict[str, Any]:
        return {"claimed": self.runtime is not None,
                "operator_key": self.operator_key.hex() if self.runtime else None,
                "runtime_id": self.runtime.runtime_id if self.runtime else None,
                "tokens_source": self.assets.source, "registry_digest": self.reg.digest}

    @staticmethod
    def claim_payload(public_key_hex: str, token: str) -> bytes:
        return core.canonical({"domain": CLAIM_DOMAIN, "key": public_key_hex,
                               "token_sha256": core.sha256_hex(token.encode())})

    def claim(self, token: Any, public_key_hex: Any, signature_hex: Any) -> Dict[str, Any]:
        with self.lock:
            if self.runtime is not None:
                raise core.ForgeError("X3_FORGE_CLAIMED", "High Command is already claimed on this Forge", "command")
            if not isinstance(token, str) or not secrets.compare_digest(token, self.setup_token):
                raise core.ForgeError("X3_FORGE_CLAIM", "setup token does not match the one the server printed",
                                      "command")
            pk = _hex(public_key_hex, 32, "operator key")
            sig = _hex(signature_hex, 64, "signature")
            verify_sig(pk, sig, self.claim_payload(public_key_hex, token), "High Command claim")
            (self.dir / "operator.json").write_text(json.dumps({"public_key": public_key_hex,
                                                                "claimed_at": int(self.clock())}))
            self._open_runtime(pk)
            return self.status()

    def _need_runtime(self) -> Runtime:
        if self.runtime is None:
            raise core.ForgeError("X3_FORGE_UNCLAIMED", "claim High Command first", "command")
        return self.runtime

    # -- compile / prove (stateless) -------------------------------------------------------------
    def compile(self, build: Any) -> Dict[str, Any]:
        with tempfile.TemporaryDirectory(prefix="forge-") as d:
            return core.compile_build(self.reg, self.assets, build, Path(d))

    def compile_and_prove(self, build: Any) -> Dict[str, Any]:
        with tempfile.TemporaryDirectory(prefix="forge-") as d:
            result = core.compile_build(self.reg, self.assets, build, Path(d))
            proof = proving.prove(result, self.assets, Path(d))
        return {"compile": result, "proof": proof}

    # -- birth -------------------------------------------------------------------------------
    def _check_connections(self, session: str, build: Dict[str, Any], normalized: Dict[str, Any]) -> List[str]:
        ids = []
        problems = []
        for c in build.get("connections") or []:
            cid = c.get("connection_id") if isinstance(c, dict) else None
            spec = self.reg.connectors.get(c.get("connector")) if isinstance(c, dict) else None
            if spec is None:
                continue
            node = f"connector.{spec['id']}"
            if not cid:
                problems.append(core.ForgeError("X3_FORGE_CONNECTION", f"{spec['name']} is not set up", node))
                continue
            try:
                rec = self.connections.get(cid, session)
            except core.ForgeError as exc:
                problems.append(core.ForgeError(exc.code, f"{spec['name']}: {exc.message}", node))
                continue
            if rec["connector"] != spec["id"] or rec.get("agent_id"):
                problems.append(core.ForgeError("X3_FORGE_CONNECTION", f"{spec['name']}: connection mismatch", node))
                continue
            ok, why = self.connections.usable(rec)
            if not ok:
                problems.append(core.ForgeError("X3_FORGE_CONNECTION", f"{spec['name']}: {why}", node))
            ids.append(cid)
        if problems:
            raise core.ForgeErrors(problems)
        return ids

    def prepare_birth(self, session: str, build: Any, agent_public_key_hex: Any) -> Dict[str, Any]:
        rt = self._need_runtime()
        pk = _hex(agent_public_key_hex, 32, "agent key")
        out = self.compile_and_prove(build)
        result, proof = out["compile"], out["proof"]
        if not proof["passed"]:
            failed = [p["id"] for p in proof["probes"] if not p["ok"]]
            raise core.ForgeErrors([core.ForgeError("X3_FORGE_PROVING", f"Proving Ground failed: {failed}",
                                                    "proving")])
        ids = self._check_connections(session, build, result["normalized"])
        manifest = core.manifest_from_dict(result["manifest"])
        # The runtime's genesis payload has no field for the build, so its nonce carries the build
        # hash: the operator's signature then binds the exact mandate, skills and connections.
        nonce = f"forge:{result['build_hash']}:{secrets.token_hex(16)}"
        payload = rt.genesis_payload(manifest, pk, nonce)
        birth_id = secrets.token_hex(16)
        cosmetics = build.get("cosmetics") if isinstance(build.get("cosmetics"), dict) else {}
        with self.lock:
            now = self.clock()
            for k in [k for k, v in self.pending.items() if v["expires"] < now]:
                self.pending.pop(k)
            self.pending[birth_id] = {"session": session, "nonce": nonce, "agent_key": pk, "payload": payload,
                                      "result": result, "proof": proof, "connection_ids": ids,
                                      "cosmetics": _clean_cosmetics(cosmetics), "expires": now + BIRTH_TTL_S}
        return {"birth_id": birth_id, "payload_hex": payload.hex(), "compile": result, "proof": proof,
                "expires_in": BIRTH_TTL_S}

    def commit_birth(self, session: str, birth_id: Any, signature_hex: Any) -> Dict[str, Any]:
        rt = self._need_runtime()
        with self.lock:
            p = self.pending.get(birth_id) if isinstance(birth_id, str) else None
            if p is None or p["session"] != session or p["expires"] < self.clock():
                raise core.ForgeError("X3_FORGE_BIRTH", "unknown or expired birth; prepare it again", "birth")
            sig = _hex(signature_hex, 64, "signature")
            result = p["result"]
            manifest = core.manifest_from_dict(result["manifest"])
            try:
                rt.genesis(manifest, p["agent_key"], p["nonce"], sig)
            except ValueError as exc:
                raise core.ForgeError("X3_FORGE_BIRTH", f"Agent Law runtime refused the birth: {exc}", "birth") from exc
            self.pending.pop(birth_id)
            agent_id = manifest.agent_id
            self.connections.bind(session, p["connection_ids"], agent_id)
            record = {
                "agent_id": agent_id,
                "name": result["normalized"]["name"],
                "born_at": int(self.clock()),
                "runtime_id": rt.runtime_id,
                "operator_key": self.operator_key.hex(),
                "agent_public_key": p["agent_key"].hex(),
                "genesis": {"nonce": p["nonce"], "payload_hex": p["payload"].hex(), "signature": signature_hex},
                "compile": result,
                "proof": p["proof"],
                "connection_ids": p["connection_ids"],
                "cosmetics": p["cosmetics"],
                "revocations": [],
                "mesh_log": [],
            }
            self._save(record)
        return self.agent(agent_id)

    # -- roster ------------------------------------------------------------------------------
    def _path(self, agent_id: str) -> Path:
        if not isinstance(agent_id, str) or not ni.registry.is_valid_near_account(agent_id):
            raise core.ForgeError("X3_FORGE_AGENT", "unknown agent", "roster")
        return self.dir / "agents" / f"{agent_id}.json"

    def _load(self, agent_id: str) -> Dict[str, Any]:
        path = self._path(agent_id)
        if not path.exists():
            raise core.ForgeError("X3_FORGE_AGENT", "unknown agent", "roster")
        return json.loads(path.read_text())

    def _save(self, record: Dict[str, Any]) -> None:
        path = self._path(record["agent_id"])
        tmp = path.with_suffix(".tmp")
        tmp.write_text(json.dumps(record, sort_keys=True, indent=1))
        tmp.replace(path)

    def _live(self, record: Dict[str, Any]) -> Dict[str, Any]:
        rt = self._need_runtime()
        snap = rt.snapshot(record["agent_id"])
        expired = snap["manifest"]["expires_at"] <= self.clock()
        status = "revoked" if snap["status"] == "revoked" else ("expired" if expired else snap["status"])
        events = [json.loads(e["payload"]) | {"seq": e["seq"], "hash": e["hash"]} for e in rt.events()]
        mine = [e for e in events if e.get("agent") == record["agent_id"]]
        return {"status": status, "runtime": snap, "events": mine}

    def agent(self, agent_id: str) -> Dict[str, Any]:
        record = self._load(agent_id)
        conns = []
        for cid in record["connection_ids"]:
            try:
                conns.append(self.connections.public(self.connections.get(cid)))
            except core.ForgeError:
                continue
        out = {k: v for k, v in record.items() if k not in ("connection_ids",)}
        out["connections"] = conns
        out.update(self._live(record))
        if out["status"] == "revoked":
            out["revoke_mandate"] = self.revoke_mandate_call(record)
        return out

    def roster(self) -> List[Dict[str, Any]]:
        out = []
        for path in sorted((self.dir / "agents").glob("*.json")):
            record = json.loads(path.read_text())
            live = self._live(record)
            out.append({"agent_id": record["agent_id"], "name": record["name"], "born_at": record["born_at"],
                        "archetype": record["compile"]["normalized"]["archetype"], "status": live["status"],
                        "stats": record["compile"]["stats"], "traits": record["compile"]["traits"],
                        "cosmetics": record["cosmetics"], "build_hash": record["compile"]["build_hash"]})
        return out

    # -- revocation (operator-signed) --------------------------------------------------------
    def revoke_payload(self, agent_id: str, reason: str, nonce: str, *, connection_id: Optional[str] = None) -> bytes:
        rt = self._need_runtime()
        body = {"domain": REVOKE_CONN_DOMAIN if connection_id else REVOKE_DOMAIN, "runtime_id": rt.runtime_id,
                "agent_id": agent_id, "reason": reason, "nonce": nonce}
        if connection_id:
            body["connection_id"] = connection_id
        return core.canonical(body)

    def _operator_check(self, record: Dict[str, Any], payload: bytes, signature_hex: Any, nonce: Any) -> None:
        if not isinstance(nonce, str) or not 16 <= len(nonce) <= 64:
            raise core.ForgeError("X3_FORGE_NONCE", "nonce must be 16..64 characters", "keys")
        if nonce in {r["nonce"] for r in record["revocations"]}:
            raise core.ForgeError("X3_FORGE_NONCE", "nonce already used", "keys")
        verify_sig(self.operator_key, _hex(signature_hex, 64, "signature"), payload, "High Command")

    def revoke(self, agent_id: str, reason: Any, nonce: Any, signature_hex: Any) -> Dict[str, Any]:
        rt = self._need_runtime()
        if not isinstance(reason, str) or not 1 <= len(reason) <= 128:
            raise core.ForgeError("X3_FORGE_REASON", "reason: 1..128 characters", "revoke")
        with self.lock:
            record = self._load(agent_id)
            payload = self.revoke_payload(agent_id, reason, nonce)
            self._operator_check(record, payload, signature_hex, nonce)
            rt.revoke(agent_id, reason)
            record["revocations"].append({"kind": "agent", "nonce": nonce, "reason": reason, "at": int(self.clock()),
                                          "signature": signature_hex, "payload_hex": payload.hex()})
            self._save(record)
        return self.agent(agent_id)

    def revoke_connection(self, agent_id: str, connection_id: Any, reason: Any, nonce: Any,
                          signature_hex: Any) -> Dict[str, Any]:
        if not isinstance(reason, str) or not 1 <= len(reason) <= 128:
            raise core.ForgeError("X3_FORGE_REASON", "reason: 1..128 characters", "revoke")
        with self.lock:
            record = self._load(agent_id)
            if connection_id not in record["connection_ids"]:
                raise core.ForgeError("X3_FORGE_CONNECTION", "connection does not belong to this agent", "comms")
            payload = self.revoke_payload(agent_id, reason, nonce, connection_id=connection_id)
            self._operator_check(record, payload, signature_hex, nonce)
            self.connections.revoke(connection_id, agent_id)
            record["revocations"].append({"kind": "connection", "connection_id": connection_id, "nonce": nonce,
                                          "reason": reason, "at": int(self.clock()), "signature": signature_hex,
                                          "payload_hex": payload.hex()})
            self._save(record)
        return self.agent(agent_id)

    @staticmethod
    def revoke_mandate_call(record: Dict[str, Any]) -> Optional[Dict[str, Any]]:
        compiled = record["compile"]["compiled"]
        if compiled is None:
            return None
        return {"method": "revoke_mandate", "args": {"agent": compiled["agent"]}, "signer": compiled["owner"],
                "attached_deposit": ni.GUARD_DEPOSITS["revoke"], "gas": ni.GUARD_GAS,
                "note": "Send this from the owner account to stop the agent on x3-guard. The Forge does not sign it."}

    # -- agent mesh: an action through a connection that this Forge enforces ------------------
    def mesh_payload(self, sender: str, to: str, kind: str, body: str, nonce: str, expires_at: int) -> bytes:
        rt = self._need_runtime()
        return core.canonical({"domain": MESH_DOMAIN, "runtime_id": rt.runtime_id, "from": sender, "to": to,
                               "type": kind, "body_sha256": core.sha256_hex(body.encode()), "nonce": nonce,
                               "expires_at": expires_at})

    def _mesh_link(self, record: Dict[str, Any]) -> Optional[Dict[str, Any]]:
        for cid in record["connection_ids"]:
            rec = self.connections.get(cid)
            if rec["connector"] == "forge_mesh":
                return rec
        return None

    def mesh_send(self, sender: Any, to: Any, kind: Any, body: Any, nonce: Any, expires_at: Any,
                  signature_hex: Any) -> Dict[str, Any]:
        """Deliver a signed agent-to-agent message, or refuse it with every reason."""
        self._need_runtime()
        with self.lock:
            record = self._load(sender)
            reasons: List[str] = []
            now = self.clock()
            if not isinstance(body, str) or len(body) > MAX_BODY:
                raise core.ForgeError("X3_FORGE_MESH", f"body: a string of at most {MAX_BODY} characters", "mesh")
            if not isinstance(kind, str) or not isinstance(to, str) or not isinstance(nonce, str) \
                    or not 16 <= len(nonce) <= 64 or type(expires_at) is not int:
                raise core.ForgeError("X3_FORGE_MESH", "malformed message", "mesh")
            payload = self.mesh_payload(sender, to, kind, body, nonce, expires_at)
            try:
                verify_sig(bytes.fromhex(record["agent_public_key"]), _hex(signature_hex, 64, "signature"), payload,
                           "agent")
            except core.ForgeError:
                reasons.append("signature")
            if expires_at < now or expires_at > now + MESH_TTL_S:
                reasons.append("expiry")
            if nonce in {m["nonce"] for m in record["mesh_log"]}:
                reasons.append("replay")
            live = self._live(record)
            if live["status"] != "active":
                reasons.append(f"agent_{live['status']}")
            manifest = live["runtime"]["manifest"]
            if "agents.message" not in manifest["capabilities"]:
                reasons.append("capability")
            if "agents.forge" not in manifest["surfaces"]:
                reasons.append("surface")
            link = self._mesh_link(record)
            if link is None:
                reasons.append("no_mesh_connection")
            else:
                ok, why = self.connections.usable(link)
                if not ok:
                    reasons.append(f"connection_{why.replace(' ', '_')}")
                if to not in link["config"].get("peers", []):
                    reasons.append("peer_not_allowed")
                if kind not in link["config"].get("messages", []):
                    reasons.append("message_type_not_allowed")
                window = [t for t in self.mesh_rate.get(sender, []) if t > now - 60]
                if len(window) >= int(link["config"].get("rate_per_min", 0)):
                    reasons.append("rate_limit")
            try:
                peer = self._load(to)
                if self._live(peer)["status"] != "active":
                    reasons.append("peer_inactive")
            except core.ForgeError:
                reasons.append("peer_unknown")
            verdict = "delivered" if not reasons else "refused"
            entry = {"nonce": nonce, "to": to, "type": kind, "at": int(now), "verdict": verdict, "reasons": reasons,
                     "body_sha256": core.sha256_hex(body.encode())}
            record["mesh_log"] = (record["mesh_log"] + [entry])[-200:]
            self._save(record)
            if verdict == "delivered":
                self.mesh_rate.setdefault(sender, []).append(now)
                inbox = self.dir / "mesh_inbox.jsonl"
                with inbox.open("a") as f:
                    f.write(json.dumps({"from": sender, **entry, "body": body}) + "\n")
            return entry

    # -- export / import ---------------------------------------------------------------------
    def export(self, agent_id: str) -> Dict[str, Any]:
        a = self.agent(agent_id)
        manifest = {
            "format": "x3-agent-forge/manifest/1",
            "agent_id": a["agent_id"],
            "name": a["name"],
            "born_at": a["born_at"],
            "status_at_export": a["status"],
            "runtime_id": a["runtime_id"],
            "operator_key": a["operator_key"],
            "agent_public_key": a["agent_public_key"],
            "genesis": a["genesis"],
            "build": a["compile"]["normalized"],
            "cosmetics": a["cosmetics"],
            "build_hash": a["compile"]["build_hash"],
            "registry_digest": a["compile"]["registry_digest"],
            "source_sha256": a["compile"]["source_sha256"],
            "compiled": a["compile"]["compiled"],
            "compiled_sha256": a["compile"]["compiled_sha256"],
            "create_mandate": a["compile"]["create_mandate"],
            "runtime_manifest": a["compile"]["manifest"],
            "stats": a["compile"]["stats"],
            "traits": a["compile"]["traits"],
            "proof": {"passed": a["proof"]["passed"], "ran_at": a["proof"]["ran_at"],
                      "probes": [{k: p[k] for k in ("id", "title", "expect", "outcome", "ok", "engine")}
                                 for p in a["proof"]["probes"]]},
            "connections": [{k: c[k] for k in ("id", "connector", "status", "confirmed_by", "config_hash")}
                            for c in a["connections"]],
            "revocations": [{k: r[k] for k in r if k != "payload_hex"} for r in a["revocations"]],
        }
        return {"manifest": manifest, "source": a["compile"]["source"]}

    def verify_import(self, manifest: Any) -> Dict[str, Any]:
        """Rebuild a character from an exported manifest alone and check every hash."""
        if not isinstance(manifest, dict) or manifest.get("format") != "x3-agent-forge/manifest/1":
            raise core.ForgeError("X3_FORGE_IMPORT", "not an X3 Agent Forge manifest", "import")
        checks: List[Dict[str, Any]] = []

        def check(name: str, ok: bool, detail: str) -> None:
            checks.append({"check": name, "ok": bool(ok), "detail": detail})

        try:
            normalized = core.normalize_build(self.reg, self.assets, core.raw_build(manifest["build"]))
        except (KeyError, TypeError, AttributeError) as exc:
            check("build_valid", False, f"malformed build: {type(exc).__name__}")
            return {"ok": False, "checks": checks}
        except core.ForgeErrors as exc:
            check("build_valid", False, "; ".join(e.message for e in exc.errors))
            return {"ok": False, "checks": checks}
        check("build_valid", True, "build passes this Forge's registry")
        digest = core.build_hash(normalized)
        check("build_hash", digest == manifest.get("build_hash") and normalized == manifest.get("build"),
              f"recomputed {digest}")
        check("registry_digest", manifest.get("registry_digest") == self.reg.digest,
              "same skill/connector registry" if manifest.get("registry_digest") == self.reg.digest
              else "built against a different registry version")
        stats = core.derive_stats(self.reg, normalized)
        check("stats", stats == manifest.get("stats"), f"recomputed {stats}")
        traits = core.derive_traits(self.reg, normalized, digest)
        check("creature", traits == manifest.get("traits"), f"seed {traits['seed'][:16]}…, body {traits['body']}")
        source, _ = core.generate_mandate_source(normalized, digest)
        src_sha = core.sha256_hex(source.encode()) if source else None
        check("mandate_source", src_sha == manifest.get("source_sha256"), f"regenerated source sha256 {src_sha}")
        compiled = manifest.get("compiled")
        if compiled is not None:
            check("compiled_binds_source", compiled["terms"].get("program_sha256") == src_sha,
                  "terms.program_sha256 equals the regenerated source hash")
            check("compiled_sha256", core.sha256_hex(core.canonical(compiled)) == manifest.get("compiled_sha256"),
                  "compiled terms hash")
        try:
            payload = bytes.fromhex(manifest["genesis"]["payload_hex"])
            body = json.loads(payload)
            check("genesis_manifest", body.get("manifest") == manifest.get("runtime_manifest"),
                  "signed genesis payload names this runtime manifest")
            check("genesis_agent_key", body.get("key") == manifest.get("agent_public_key"),
                  "signed genesis payload names this agent key")
            check("genesis_binds_build", body.get("nonce", "").startswith(f"forge:{digest}:"),
                  "the signed genesis nonce carries this build hash")
            verify_sig(bytes.fromhex(manifest["operator_key"]), bytes.fromhex(manifest["genesis"]["signature"]),
                       payload, "genesis")
            check("genesis_signature", True, "operator signature over the genesis payload verifies")
        except (KeyError, ValueError, TypeError, core.ForgeError) as exc:
            check("genesis_signature", False, f"{type(exc).__name__}: {getattr(exc, 'message', exc)}")
        return {"ok": all(c["ok"] for c in checks), "checks": checks, "traits": traits, "stats": stats,
                "normalized": normalized, "cosmetics": manifest.get("cosmetics") or {}}


COSMETIC_KEYS = {"palette": str, "pattern": str, "idle": str, "glow": str}


def _clean_cosmetics(raw: Dict[str, Any]) -> Dict[str, Any]:
    """Cosmetics change nothing about permissions; keep only short known strings."""
    return {k: v[:32] for k, v in raw.items() if k in COSMETIC_KEYS and isinstance(v, str)}
