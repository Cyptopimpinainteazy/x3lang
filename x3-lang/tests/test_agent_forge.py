"""X3 Agent Forge: registry, build → .x3 → real compile, Proving Ground, birth, revocation,
connections, mesh, export/import and the HTTP host. Every enforcement verdict asserted here
comes from the repository's own checkers (near_mandate, near_intents, agent_constitution,
agent_runtime)."""
import copy
import http.client
import json
import secrets
import threading
from datetime import datetime, timezone
from http.server import ThreadingHTTPServer

import pytest
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from cryptography.hazmat.primitives import serialization

from forge import core, proving
from forge.connections import ConnectionStore, check_url, classify
from forge.server import Api, make_handler
from forge.store import Forge

NOW = datetime(2026, 11, 14, 18, 0, tzinfo=timezone.utc)


def raw(pub) -> str:
    return pub.public_bytes(serialization.Encoding.Raw, serialization.PublicFormat.Raw).hex()


@pytest.fixture(scope="module")
def reg():
    return core.load_registry()


@pytest.fixture(scope="module")
def assets():
    return core.load_assets()


def trader_build(**over):
    b = {
        "version": 1, "name": "Vexa", "archetype": "trader",
        "identity": {"owner": "x3-owner.near", "agent": "vexa.x3-owner.near", "expires": "30d", "budget": 1000},
        "skills": {"read_balances": {}, "market_watch": {}, "quote_hunter": {}, "spend": {}, "payees": {},
                   "cosign": {"approvers": ["x3-owner.near"]}, "hive_voice": {}, "brood": {"spawn_limit": 2}},
        "connections": [{"connector": "x3_general"}, {"connector": "forge_mesh"}],
        "cosmetics": {"palette": "acid"},
    }
    b.update(over)
    return b


def compile_(reg, assets, build, tmp_path):
    return core.compile_build(reg, assets, build, tmp_path, now=NOW)


# -- registry -------------------------------------------------------------------------------

def test_registry_loads_and_rejects_bad_entries(reg):
    assert {"spend", "payees", "cosign"} <= set(reg.skills)
    skills = json.loads((core.REGISTRY_DIR / "skills.json").read_text())
    arch = json.loads((core.REGISTRY_DIR / "archetypes.json").read_text())
    conns = json.loads((core.REGISTRY_DIR / "connectors.json").read_text())
    bad = copy.deepcopy(skills)
    bad[0]["risk"] = 9
    with pytest.raises(core.ForgeError, match="risk"):
        core.validate_registry(arch, bad, conns)
    bad = copy.deepcopy(skills)
    bad[0]["requires"] = ["self_modify_mandate"]
    with pytest.raises(core.ForgeError, match="unknown skill"):
        core.validate_registry(arch, bad, conns)
    bad = copy.deepcopy(skills)
    bad[0]["compiles_to"] = {"kind": "mandate", "clauses": []}
    with pytest.raises(core.ForgeError, match="only spend"):
        core.validate_registry(arch, bad, conns)
    cyc = copy.deepcopy(skills)
    by = {s["id"]: s for s in cyc}
    by["read_balances"]["requires"] = ["spend"]
    with pytest.raises(core.ForgeError, match="cycle"):
        core.validate_registry(arch, cyc, conns)
    badc = copy.deepcopy(conns)
    badc[0]["setup_url"] = "http://evil.example/"
    with pytest.raises(core.ForgeError, match="setup_url"):
        core.validate_registry(arch, skills, badc)


# -- build → .x3 → compile ------------------------------------------------------------------

def test_build_compiles_through_the_real_mandate_compiler(reg, assets, tmp_path):
    r = compile_(reg, assets, trader_build(), tmp_path)
    assert r["source"].splitlines()[2] == "mandate forge_vexa {"
    assert "per_payment 50000000" in r["source"] and "approve above 30000000 by 1 of x3-owner.near" in r["source"]
    terms = r["compiled"]["terms"]
    assert terms["program_sha256"] == r["source_sha256"]
    assert terms["per_payment_cap"] == "50000000" and terms["daily_cap"] == "200000000"
    assert terms["recipients"] == ["x3-demo.near"]
    # The same lowering as `near_intents.py mandate args` on the same file.
    import near_intents as ni
    p = tmp_path / "same.x3"
    p.write_text(r["source"])
    assert ni.mandates().compile_mandate(str(p), assets.tokens, now=NOW) == r["compiled"]
    assert r["create_mandate"]["method"] == "create_mandate" and r["create_mandate"]["signer"] == "x3-owner.near"
    m = r["manifest"]
    assert m["agent_id"] == "vexa.x3-owner.near" and m["spawn_limit"] == 2
    assert {"pay.spend", "agents.delegate", "agents.message"} <= set(m["capabilities"])
    assert m["surfaces"] == ["agents.forge", "pay.guard"]


def test_cosmetics_never_reach_the_hash_mandate_or_manifest(reg, assets, tmp_path):
    a = compile_(reg, assets, trader_build(cosmetics={"palette": "acid"}), tmp_path)
    b = compile_(reg, assets, trader_build(cosmetics={"palette": "void", "pattern": "stripes"}), tmp_path)
    assert a["build_hash"] == b["build_hash"] and a["source"] == b["source"] and a["manifest"] == b["manifest"]


def test_changing_one_cap_changes_hash_source_and_creature(reg, assets, tmp_path):
    a = compile_(reg, assets, trader_build(), tmp_path)
    build = trader_build()
    build["skills"]["spend"] = {"per_payment": "49"}
    b = compile_(reg, assets, build, tmp_path)
    assert a["build_hash"] != b["build_hash"]
    assert a["compiled"]["terms"]["program_sha256"] != b["compiled"]["terms"]["program_sha256"]
    assert a["traits"]["seed"] != b["traits"]["seed"]


@pytest.mark.parametrize("mutate,node", [
    (lambda b: b["skills"].update({"self_modify_mandate": {}}), "skill.self_modify_mandate"),
    (lambda b: b["skills"].pop("payees"), "skill.spend"),
    (lambda b: b["skills"].pop("read_balances"), "skill.spend"),
    (lambda b: b["skills"]["spend"].update({"per_payment": "0"}), "skill.spend.per_payment"),
    (lambda b: b["skills"]["spend"].update({"per_payment": "0.0000001"}), "skill.spend.per_payment"),
    (lambda b: b["skills"]["spend"].update({"asset": "Solana.DOGE"}), "skill.spend.asset"),
    (lambda b: b["skills"]["spend"].update({"refund_to": "x3-owner.near"}), "skill.spend.refund_to"),
    (lambda b: b["skills"]["spend"].update({"cooldown": "5m"}), "skill.spend.cooldown"),
    (lambda b: b["skills"]["payees"].update({"recipients": ["sender"]}), "skill.payees.recipients"),
    (lambda b: b.update({"connections": []}), "comms"),
    (lambda b: b.update({"connections": [{"connector": "super_ide"}]}), "connector.super_ide"),
    (lambda b: b["identity"].update({"agent": "x3-owner.near"}), "origin.agent"),
    (lambda b: b.update({"archetype": "engineer"}), "skill.spend"),
])
def test_bad_builds_stop_with_the_node_that_caused_them(reg, assets, tmp_path, mutate, node):
    b = trader_build()
    mutate(b)
    with pytest.raises(core.ForgeErrors) as exc:
        compile_(reg, assets, b, tmp_path)
    assert node in [e.node for e in exc.value.errors], exc.value.to_list()


def test_compiler_errors_map_back_to_the_skill(reg, assets, tmp_path):
    b = trader_build()
    b["skills"]["spend"] = {"per_payment": "300", "per_24h": "200"}
    b["skills"]["cosign"] = {"above": "20", "approvers": ["x3-owner.near"]}
    with pytest.raises(core.ForgeErrors) as exc:
        compile_(reg, assets, b, tmp_path)
    err = exc.value.errors[0]
    assert err.code == "X3_MANDATE_LIMIT" and err.node == "skill.spend.per_payment"
    b = trader_build()
    b["skills"]["cosign"] = {"above": "50", "approvers": ["x3-owner.near"]}  # not below per_payment
    with pytest.raises(core.ForgeErrors) as exc:
        compile_(reg, assets, b, tmp_path)
    assert exc.value.errors[0].code == "X3_MANDATE_APPROVAL" and exc.value.errors[0].node == "skill.cosign"
    b = trader_build()
    b["skills"]["cosign"] = {"approvers": ["vexa.x3-owner.near"]}  # agent approving itself
    with pytest.raises(core.ForgeErrors) as exc:
        compile_(reg, assets, b, tmp_path)
    assert exc.value.errors[0].code == "X3_MANDATE_APPROVAL"


def test_agent_without_spend_has_no_mandate(reg, assets, tmp_path):
    b = {"version": 1, "name": "Gantry", "archetype": "engineer",
         "identity": {"owner": "x3-owner.near", "agent": "gantry.x3-owner.near", "expires": "7d", "budget": 50},
         "skills": {"code_read": {}, "patch_smith": {}, "test_runner": {}},
         "connections": [{"connector": "super_ide"}]}
    r = compile_(reg, assets, b, tmp_path)
    assert r["source"] is None and r["compiled"] is None and r["create_mandate"] is None
    p = proving.prove(r, assets, tmp_path, now=NOW)
    assert p["passed"] and p["probes"][0]["id"] == "no_spend"


# -- Proving Ground -------------------------------------------------------------------------

def test_proving_ground_uses_the_guard_checks(reg, assets, tmp_path):
    r = compile_(reg, assets, trader_build(), tmp_path)
    p = proving.prove(r, assets, tmp_path, now=NOW)
    by = {x["id"]: x for x in p["probes"]}
    assert p["passed"], [(x["id"], x["outcome"]) for x in p["probes"] if not x["ok"]]
    assert by["permitted"]["outcome"] == "allow"
    failed = lambda pid: {c["guard"] for c in by[pid]["checks"] if not c["ok"]}  # noqa: E731
    assert failed("forbidden_recipient") == {"mandate_recipient"}
    assert "mandate_per_payment" in failed("over_cap")
    assert failed("refund_hijack") == {"mandate_refund_to"}
    assert "mandate_active" in failed("expired")
    assert failed("over_daily") == {"mandate_daily"}
    assert "mandate_active" in failed("revoked")
    assert by["needs_approval"]["outcome"] == "allow" and by["needs_approval"]["approvals_needed"] == 1
    assert by["child_within"]["outcome"] == "allow"
    assert "capabilities" in failed("child_escalate_capability")
    assert "surfaces" in failed("child_escalate_surface")
    assert "approval" in failed("child_unapproved")


def test_without_brood_a_child_is_refused(reg, assets, tmp_path):
    b = trader_build()
    b["skills"].pop("brood")
    r = compile_(reg, assets, b, tmp_path)
    p = proving.prove(r, assets, tmp_path, now=NOW)
    by = {x["id"]: x for x in p["probes"]}
    assert p["passed"] and by["child_within"]["outcome"] == "deny"


# -- connections ----------------------------------------------------------------------------

def test_url_allowlist():
    assert check_url("https://ide.dotx3.net/")
    for bad in ("http://ide.dotx3.net/", "https://evil.example/", "https://dotx3.net.evil.example/",
                "https://user:pw@ide.dotx3.net/", "https://127.0.0.1/", "javascript:alert(1)"):
        with pytest.raises(core.ForgeError):
            check_url(bad)
    assert check_url("http://localhost:3000/mcp", local=True)
    with pytest.raises(core.ForgeError):
        check_url("http://ide.dotx3.net/", local=True)


def test_health_classification():
    assert classify(200, {})[0] == "ok"
    assert classify(302, {"location": "https://x3.cloudflareaccess.com/cdn-cgi/access/login"})[0] == "login_gated"
    assert classify(403, {})[0] == "login_gated"
    assert classify(302, {"location": "https://elsewhere.example/"})[0] == "failed"
    assert classify(500, {})[0] == "failed"


def test_setup_state_is_one_time_and_session_bound(reg, tmp_path):
    store = ConnectionStore(reg, tmp_path, transport=lambda url: (200, {}))
    out = store.start("sess-a", "super_ide", {}, None, "http://forge/api/connect/callback")
    cid, state = out["connection"]["id"], out["state"]
    assert out["open_url"].startswith("https://ide.dotx3.net/?x3_forge_state=")
    with pytest.raises(core.ForgeError, match="unknown connection"):
        store.callback("sess-b", state, cid)
    with pytest.raises(core.ForgeError, match="unknown, used or expired"):
        store.callback("sess-a", state + "x", cid)
    assert store.callback("sess-a", state, cid)["confirmed_by"] == "page"
    with pytest.raises(core.ForgeError, match="unknown, used or expired"):
        store.callback("sess-a", state, cid)


def test_secrets_stay_server_side(reg, tmp_path):
    store = ConnectionStore(reg, tmp_path, transport=lambda url: (200, {}))
    out = store.start("s", "custom_api", {"url": "https://api.dotx3.net/v1"}, None, "http://f/cb")
    secret = "sk-live-" + secrets.token_hex(16)
    pub = store.set_secret("s", out["connection"]["id"], secret)
    assert pub["has_secret"] and secret not in json.dumps(pub)
    stored = (tmp_path / "connections" / f"{pub['id']}.json").read_text()
    assert secret not in stored
    assert (tmp_path / "secrets" / pub["id"]).read_text() == secret
    assert ((tmp_path / "secrets" / pub["id"]).stat().st_mode & 0o777) == 0o600


# -- the full lifecycle ----------------------------------------------------------------------

class Owner:
    def __init__(self):
        self.key = Ed25519PrivateKey.generate()
        self.pub = raw(self.key.public_key())

    def sign(self, payload_hex: str) -> str:
        return self.key.sign(bytes.fromhex(payload_hex)).hex()


def claimed_forge(reg, assets, tmp_path, transport=lambda url: (200, {})):
    forge = Forge(tmp_path / "data", reg, assets, transport=transport, setup_token="tok")
    owner = Owner()
    with pytest.raises(core.ForgeError, match="setup token"):
        forge.claim("wrong", owner.pub, owner.sign(forge.claim_payload(owner.pub, "wrong").hex()))
    forge.claim("tok", owner.pub, owner.sign(forge.claim_payload(owner.pub, "tok").hex()))
    with pytest.raises(core.ForgeError, match="already claimed"):
        forge.claim("tok", owner.pub, owner.sign(forge.claim_payload(owner.pub, "tok").hex()))
    return forge, owner


def plug(forge, session, connector, config=None, agent="vexa.x3-owner.near"):
    out = forge.connections.start(session, connector, config or {}, agent, "http://f/api/connect/callback")
    cid = out["connection"]["id"]
    if out["state"]:
        forge.connections.confirm(session, out["state"], cid)
    forge.connections.health(session, cid)
    return cid


def birth(forge, owner, session, build):
    agent_key = Ed25519PrivateKey.generate()
    prep = forge.prepare_birth(session, build, raw(agent_key.public_key()))
    rec = forge.commit_birth(session, prep["birth_id"], owner.sign(prep["payload_hex"]))
    return rec, agent_key


def test_birth_revoke_mesh_export_import(reg, assets, tmp_path):
    forge, owner = claimed_forge(reg, assets, tmp_path)
    s = "session-1"
    mesh_cfg = {"peers": ["gantry.x3-owner.near"], "messages": ["status"], "rate_per_min": 5}
    build = trader_build(connections=[{"connector": "x3_general", "connection_id": plug(forge, s, "x3_general",
                                                                                         {"guard_contract": "x3-guard.near"})},
                                      {"connector": "forge_mesh", "connection_id": plug(forge, s, "forge_mesh", mesh_cfg)}])
    # A forged operator signature is refused by the runtime.
    prep = forge.prepare_birth(s, build, raw(Ed25519PrivateKey.generate().public_key()))
    with pytest.raises(core.ForgeError, match="refused the birth"):
        forge.commit_birth(s, prep["birth_id"], Ed25519PrivateKey.generate().sign(bytes.fromhex(prep["payload_hex"])).hex())
    vexa, vexa_key = birth(forge, owner, s, build)
    assert vexa["status"] == "active" and vexa["events"][0]["kind"] == "born"

    peer_build = {"version": 1, "name": "Gantry", "archetype": "engineer",
                  "identity": {"owner": "x3-owner.near", "agent": "gantry.x3-owner.near", "expires": "7d",
                               "budget": 50},
                  "skills": {"code_read": {}, "hive_voice": {}},
                  "connections": [{"connector": "forge_mesh", "connection_id": plug(
                      forge, s, "forge_mesh", {"peers": ["vexa.x3-owner.near"], "messages": ["status"],
                                               "rate_per_min": 5}, agent="gantry.x3-owner.near")}]}
    birth(forge, owner, s, peer_build)

    def send(to="gantry.x3-owner.near", kind="status", key=vexa_key, nonce=None):
        nonce = nonce or secrets.token_hex(12)
        exp = int(forge.clock()) + 60
        payload = forge.mesh_payload("vexa.x3-owner.near", to, kind, "hello", nonce, exp)
        return forge.mesh_send("vexa.x3-owner.near", to, kind, "hello", nonce, exp, key.sign(payload).hex())

    assert send()["verdict"] == "delivered"
    assert "message_type_not_allowed" in send(kind="incident")["reasons"]
    assert "signature" in send(key=Ed25519PrivateKey.generate())["reasons"]
    n = secrets.token_hex(12)
    send(nonce=n)
    assert "replay" in send(nonce=n)["reasons"]

    # Revoke the mesh connection (operator-signed) → messages through it stop.
    mesh_cid = build["connections"][1]["connection_id"]
    nonce = secrets.token_hex(12)
    payload = forge.revoke_payload("vexa.x3-owner.near", "rotate", nonce, connection_id=mesh_cid)
    with pytest.raises(core.ForgeError, match="signature"):
        forge.revoke_connection("vexa.x3-owner.near", mesh_cid, "rotate", nonce,
                                Ed25519PrivateKey.generate().sign(payload).hex())
    forge.revoke_connection("vexa.x3-owner.near", mesh_cid, "rotate", nonce, owner.sign(payload.hex()))
    assert "connection_revoked" in send()["reasons"]

    # Export → import reproduces hashes, stats and creature; tampering is caught.
    exported = forge.export("vexa.x3-owner.near")
    blob = json.dumps(exported)
    assert "session-1" not in blob and '"session"' not in blob
    report = forge.verify_import(exported["manifest"])
    assert report["ok"], [c for c in report["checks"] if not c["ok"]]
    tampered = copy.deepcopy(exported["manifest"])
    tampered["build"]["skills"]["spend"]["per_payment"]["base_units"] = "60000000"
    tampered["build"]["skills"]["spend"]["per_payment"]["amount"] = "60"
    assert not forge.verify_import(tampered)["ok"]

    # Revoke the agent → runtime says revoked, later messages refused, revoke_mandate printed.
    nonce = secrets.token_hex(12)
    payload = forge.revoke_payload("vexa.x3-owner.near", "retired", nonce)
    out = forge.revoke("vexa.x3-owner.near", "retired", nonce, owner.sign(payload.hex()))
    assert out["status"] == "revoked" and out["runtime"]["status"] == "revoked"
    assert out["revoke_mandate"]["method"] == "revoke_mandate"
    assert any(e["kind"] == "revoked" for e in out["events"])
    assert "agent_revoked" in send()["reasons"]
    with pytest.raises(core.ForgeError, match="nonce already used"):
        forge.revoke("vexa.x3-owner.near", "retired", nonce, owner.sign(payload.hex()))


def test_birth_refuses_unhealthy_connections_and_failed_proofs(reg, assets, tmp_path):
    forge, owner = claimed_forge(reg, assets, tmp_path, transport=lambda url: (500, {}))
    s = "s"
    cid = plug(forge, s, "x3_general", {"guard_contract": "x3-guard.near"})
    build = trader_build(connections=[{"connector": "x3_general", "connection_id": cid}])
    build["skills"].pop("hive_voice"), build["skills"].pop("brood")
    with pytest.raises(core.ForgeErrors) as exc:
        forge.prepare_birth(s, build, raw(Ed25519PrivateKey.generate().public_key()))
    assert exc.value.errors[0].node == "connector.x3_general"
    # Another session cannot use this session's connection.
    forge2 = forge
    with pytest.raises(core.ForgeErrors):
        forge2.prepare_birth("other", build, raw(Ed25519PrivateKey.generate().public_key()))


def test_secret_never_appears_in_exports(reg, assets, tmp_path):
    forge, owner = claimed_forge(reg, assets, tmp_path)
    s = "s"
    out = forge.connections.start(s, "custom_api", {"url": "https://api.dotx3.net/"}, None, "http://f/cb")
    cid = out["connection"]["id"]
    secret = "sk-" + secrets.token_hex(20)
    forge.connections.set_secret(s, cid, secret)
    forge.connections.health(s, cid)
    build = {"version": 1, "name": "Scopa", "archetype": "scout",
             "identity": {"owner": "x3-owner.near", "agent": "scopa.x3-owner.near", "expires": "1d", "budget": 5},
             "skills": {"read_balances": {}}, "connections": [{"connector": "custom_api", "connection_id": cid}]}
    birth(forge, owner, s, build)
    everything = json.dumps(forge.export("scopa.x3-owner.near")) + json.dumps(forge.agent("scopa.x3-owner.near")) \
        + json.dumps(forge.roster())
    assert secret not in everything
    for path in (tmp_path / "data").rglob("*"):
        if path.is_file() and "secrets" not in path.parts:
            assert secret.encode() not in path.read_bytes(), path


# -- HTTP host ------------------------------------------------------------------------------

@pytest.fixture
def http_forge(reg, assets, tmp_path):
    forge = Forge(tmp_path / "data", reg, assets, transport=lambda url: (200, {}), setup_token="tok")
    srv = ThreadingHTTPServer(("127.0.0.1", 0), make_handler(Api(forge), web_dist=tmp_path / "nodist"))
    t = threading.Thread(target=srv.serve_forever, daemon=True)
    t.start()
    yield forge, srv.server_address[1]
    srv.shutdown()


def call(port, method, path, body=None, headers=None):
    c = http.client.HTTPConnection("127.0.0.1", port, timeout=10)
    h = {"Content-Type": "application/json"} if body is not None else {}
    h.update(headers or {})
    c.request(method, path, json.dumps(body) if body is not None else None, h)
    r = c.getresponse()
    data = r.read()
    return r.status, dict(r.getheaders()), (json.loads(data) if data and r.getheader("Content-Type", "").startswith(
        "application/json") else data)


def test_http_compile_errors_csrf_and_callback(http_forge):
    forge, port = http_forge
    st, hdr, out = call(port, "POST", "/api/compile", {"build": trader_build()})
    assert st == 200 and out["compiled"]["terms"]["per_payment_cap"] == "50000000"
    assert "default-src 'self'" in hdr["Content-Security-Policy"] and hdr["Referrer-Policy"] == "no-referrer"
    cookie = hdr["Set-Cookie"].split(";")[0]
    st, _, out = call(port, "POST", "/api/compile", {"build": trader_build(archetype="nope")})
    assert st == 400 and out["errors"][0]["node"] == "origin.archetype"
    st, _, _ = call(port, "POST", "/api/compile", {"build": trader_build()}, {"Origin": "https://evil.example"})
    assert st == 403
    c = http.client.HTTPConnection("127.0.0.1", port)
    c.request("POST", "/api/compile", "build=1", {"Content-Type": "application/x-www-form-urlencoded"})
    assert c.getresponse().status == 415
    st, _, out = call(port, "POST", "/api/connect/start", {"connector": "super_ide"}, {"Cookie": cookie})
    cid, state = out["connection"]["id"], out["state"]
    st, hdr, _ = call(port, "GET", f"/api/connect/callback?connection={cid}&state=forged", headers={"Cookie": cookie})
    assert st == 303 and "link_error" in hdr["Location"]
    st, hdr, _ = call(port, "GET", f"/api/connect/callback?connection={cid}&state={state}", headers={"Cookie": cookie})
    assert st == 303 and hdr["Location"] == f"/?linked={cid}#comms"
    st, _, out = call(port, "GET", f"/api/connect/{cid}", headers={"Cookie": cookie})
    assert out["status"] == "linked" and out["confirmed_by"] == "page" and "session" not in out
    st, _, _ = call(port, "GET", "/api/nope")
    assert st == 404


# -- Cloudflare Access at the origin ----------------------------------------------------------

import base64 as _b64
from cryptography.hazmat.primitives.asymmetric import padding as _pad, rsa as _rsa
from cryptography.hazmat.primitives import hashes as _hashes
from forge.access import AccessVerifier

TEAM = "https://x3star.cloudflareaccess.com"
AUD = "a" * 64


def _jwt(key, kid="k1", **claims):
    enc = lambda d: _b64.urlsafe_b64encode(json.dumps(d).encode()).rstrip(b"=").decode()  # noqa: E731
    body = {"iss": TEAM, "aud": [AUD], "email": "Owner@Example.com", "exp": 2_000_000_000, "nbf": 1_000}
    body.update(claims)
    head = enc({"alg": "RS256", "kid": kid})
    signing = f"{head}.{enc(body)}"
    sig = key.sign(signing.encode(), _pad.PKCS1v15(), _hashes.SHA256())
    return f"{signing}.{_b64.urlsafe_b64encode(sig).rstrip(b'=').decode()}"


def _jwks(key, kid="k1"):
    n = key.public_key().public_numbers()
    b = lambda i: _b64.urlsafe_b64encode(i.to_bytes((i.bit_length() + 7) // 8, "big")).rstrip(b"=").decode()  # noqa: E731
    return {"keys": [{"kty": "RSA", "kid": kid, "n": b(n.n), "e": b(n.e)}]}


def test_access_verifier_matches_the_x3star_origin_rules():
    key = _rsa.generate_private_key(public_exponent=65537, key_size=2048)
    other = _rsa.generate_private_key(public_exponent=65537, key_size=2048)
    v = AccessVerifier(TEAM, AUD, "owner@example.com", fetch=lambda url: _jwks(key), clock=lambda: 1_700_000_000)
    assert v.verify(_jwt(key)) == (True, "owner@example.com")
    assert v.verify(None)[1] == "missing token"
    assert v.verify(_jwt(other))[1] == "bad signature"
    assert v.verify(_jwt(key, aud=["b" * 64]))[1] == "wrong audience"
    assert v.verify(_jwt(key, iss="https://evil.cloudflareaccess.com"))[1] == "wrong issuer"
    assert v.verify(_jwt(key, email="someone@example.com"))[1] == "wrong owner"
    assert v.verify(_jwt(key, exp=1_600_000_000))[1] == "expired"
    assert v.verify(_jwt(key, kid="k2"))[1] == "unknown key id"
    none = _jwt(key).split(".")
    none[0] = _b64.urlsafe_b64encode(b'{"alg":"none","kid":"k1"}').rstrip(b"=").decode()
    assert v.verify(".".join(none))[1] == "unsupported algorithm"
    with pytest.raises(ValueError):
        AccessVerifier("https://evil.example", AUD, "owner@example.com")


def test_origin_refuses_requests_without_the_owner_jwt(reg, assets, tmp_path):
    key = _rsa.generate_private_key(public_exponent=65537, key_size=2048)
    v = AccessVerifier(TEAM, AUD, "owner@example.com", fetch=lambda url: _jwks(key))
    forge = Forge(tmp_path / "data", reg, assets, transport=lambda url: (200, {}), setup_token="tok")
    srv = ThreadingHTTPServer(("127.0.0.1", 0), make_handler(Api(forge), web_dist=tmp_path / "nodist", access=v))
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    port = srv.server_address[1]
    try:
        assert call(port, "GET", "/health")[0] == 200
        assert call(port, "GET", "/api/status")[0] == 403
        assert call(port, "GET", "/")[0] == 403
        assert call(port, "GET", "/api/status", headers={"Cf-Access-Jwt-Assertion": _jwt(key, email="x@y.com")})[0] == 403
        assert call(port, "GET", "/api/status", headers={"Cf-Access-Jwt-Assertion": _jwt(key)})[0] == 200
    finally:
        srv.shutdown()
