"""Agent Forge × X3 GPU mesh, end to end over HTTP.

A real gpu_mesh coordinator and worker, a test-only stand-in for the contributor's Ollama, and the
Forge gate and runner. A born agent's signed task goes Forge gate → coordinator → worker → "model",
and the model's .x3 is judged by the real compiler and the agent's own compiled mandate
(`near_agent.run`, unchanged). Every refusal below comes from that code, not from the test."""
from __future__ import annotations

import json
import secrets
import threading
import time
import pytest
from cryptography.hazmat.primitives import serialization
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey

from forge import core
from forge_mesh_support import MODEL, ProgramWritingOllama
from forge.store import Forge
from gpu_mesh.coordinator import Config, Coordinator, serve
from gpu_mesh.protocol import Identity
from gpu_mesh.store import Store
from gpu_mesh.worker import OllamaClient, Worker

OWNER, AGENT = "x3-owner.near", "vexa.x3-owner.near"


def raw(pub) -> str:
    return pub.public_bytes(serialization.Encoding.Raw, serialization.PublicFormat.Raw).hex()


@pytest.fixture
def system(tmp_path):
    store = Store(str(tmp_path / "mesh.db"))
    identity = Identity.generate()
    coord = Coordinator(store, identity, Config(poll_wait_s=1, pickup_timeout_s=3, job_timeout_s=15,
                                                heartbeat_ttl_s=30, canary_interval_s=3600))
    server = serve(coord, "127.0.0.1", 0)
    mesh_url = f"http://127.0.0.1:{server.server_address[1]}"
    threading.Thread(target=server.serve_forever, daemon=True).start()
    ollama = ProgramWritingOllama()
    wid = Identity.generate()
    store.enroll_fleet(wid.public_hex, "x3gpu1")
    worker = Worker(wid, mesh_url, OllamaClient(ollama.url), label="x3gpu1", heartbeat_s=0.5,
                    gpus=lambda: [{"name": "RTX 3090", "vendor": "nvidia", "vram_mb": 24576}], idle_wait_s=0.2)
    worker.start()
    deadline = time.time() + 10
    while worker.status != "approved" and time.time() < deadline:
        time.sleep(0.05)
    assert worker.status == "approved"
    app_key = store.add_app_key("forge", allow_public=False, requests_per_minute=600, daily_token_budget=1_000_000,
                                max_tokens_cap=4096)

    reg, assets = core.load_registry(), core.load_assets()
    forge = Forge(tmp_path / "forge", reg, assets, setup_token="tok")
    owner = Ed25519PrivateKey.generate()
    pk = raw(owner.public_key())
    forge.claim("tok", pk, owner.sign(forge.claim_payload(pk, "tok")).hex())
    yield {"forge": forge, "owner": owner, "mesh_url": mesh_url, "app_key": app_key, "ollama": ollama,
           "store": store, "identity": identity}
    worker.stop()
    worker.join(3)
    server.shutdown()
    server.server_close()
    ollama.close()


def born_agent(s, *, daily_tokens=100_000, max_tokens=1500, spend=True):
    forge = s["forge"]
    session = "desk"
    out = forge.connections.start(session, "gpu_mesh", {"url": s["mesh_url"]}, AGENT, "http://f/cb")
    cid = out["connection"]["id"]
    forge.connections.set_secret(session, cid, s["app_key"])
    health = forge.connections.health(session, cid)
    assert health["status"] == "linked" and health["health"]["status"] == "ok", health
    skills = {"read_balances": {}, "mesh_mind": {"models": [MODEL], "max_tokens": max_tokens,
                                                 "daily_tokens": daily_tokens}}
    if spend:
        skills.update({"spend": {}, "payees": {}})
    build = {"version": 1, "name": "Vexa", "archetype": "trader",
             "identity": {"owner": OWNER, "agent": AGENT, "expires": "30d", "budget": 1000},
             "skills": skills, "connections": [{"connector": "gpu_mesh", "connection_id": cid}]}
    agent_key = Ed25519PrivateKey.generate()
    prep = forge.prepare_birth(session, build, raw(agent_key.public_key()))
    assert {p["id"] for p in prep["proof"]["probes"]} >= {"mind_within", "mind_other_model", "mind_over_budget",
                                                        "mind_revoked", "mind_unsigned"}
    assert prep["proof"]["passed"]
    forge.commit_birth(session, prep["birth_id"], s["owner"].sign(bytes.fromhex(prep["payload_hex"])).hex())
    return agent_key, cid


def signed_run(forge, key, task, *, model=MODEL, max_tokens=1500, nonce=None, sign_with=None):
    nonce = nonce or secrets.token_hex(12)
    exp = int(forge.clock()) + 60
    body = {"model": model, "task": task, "max_tokens": max_tokens}
    payload = forge.infer_payload(AGENT, "X3/FORGE/RUN/1", nonce, exp, body)
    sig = (sign_with or key).sign(payload).hex()
    return forge.run(AGENT, task, model, max_tokens, nonce, exp, sig)


def test_agent_thinks_on_the_mesh_and_the_mandate_decides(system):
    forge = system["forge"]
    key, _ = born_agent(system)
    out = signed_run(forge, key, "Pay x3-demo.near 10 USDC for invoice 77.")
    assert out["verdict"] == "answered", out
    report = out["result"]
    assert report["status"] == "allowed" and report["untrusted_output"] is True
    stages = {st["stage"]: st["ok"] for st in report["stages"]}
    assert stages == {"model": True, "compile": True, "under_mandate": True, "nonce": True, "mandate": True}
    assert report["effect"]["recipient"] == "x3-demo.near" and report["effect"]["amount"] == 10_000_000
    assert report["model"]["provider"] == "x3-gpu-mesh"
    assert out["entry"]["tokens"] == 18 and out["budget"]["used_24h"] == 18
    assert len(out["entry"]["mesh_job"]) == 32
    # The coordinator kept a signed, hash-chained receipt for the job.
    assert system["store"].verify_receipts(system["identity"].public_hex)["receipts"] == 1

    # A hostile or confused model: the mesh answers, the mandate refuses.
    system["ollama"].recipient = "attacker.near"
    bad = signed_run(forge, key, "Pay x3-demo.near 10 USDC.")
    assert bad["verdict"] == "answered" and bad["result"]["status"] == "rejected"
    mandate = next(st for st in bad["result"]["stages"] if st["stage"] == "mandate")
    assert "mandate_recipient" in [c["guard"] for c in mandate["mandate"]["checks"] if not c["ok"]]

    system["ollama"].recipient, system["ollama"].amount = "x3-demo.near", 75_000_000
    over = signed_run(forge, key, "Pay x3-demo.near 75 USDC.")
    assert over["result"]["status"] == "rejected"


def test_gate_refuses_before_the_mesh_is_called(system):
    forge, ollama = system["forge"], system["ollama"]
    key, cid = born_agent(system, daily_tokens=1000, max_tokens=1000)
    calls = len(ollama.calls)
    forged = signed_run(forge, key, "Pay x3-demo.near 1 USDC.", sign_with=Ed25519PrivateKey.generate())
    assert forged["verdict"] == "refused" and "signature" in forged["reasons"]
    other = signed_run(forge, key, "Pay x3-demo.near 1 USDC.", model="llama3:70b", max_tokens=100)
    assert other["reasons"] == ["model_not_allowed"]
    too_big = signed_run(forge, key, "Pay x3-demo.near 1 USDC.", max_tokens=2000)
    assert "max_tokens_over_cap" in too_big["reasons"]
    assert len(ollama.calls) == calls, "refused requests must never reach a GPU"

    ok = signed_run(forge, key, "Pay x3-demo.near 1 USDC.", max_tokens=900)
    assert ok["verdict"] == "answered"
    n = ok["entry"]["nonce"]
    assert signed_run(forge, key, "again", max_tokens=100, nonce=n)["reasons"] == ["replay"]
    spent = signed_run(forge, key, "Pay x3-demo.near 1 USDC.", max_tokens=1000)
    assert spent["reasons"] == ["budget_exhausted"]  # 18 used + 1000 requested > 1000 a day

    # Revoke the GPU-mesh connection (owner-signed): the agent loses its model access.
    nonce = secrets.token_hex(12)
    payload = forge.revoke_payload(AGENT, "rotate key", nonce, connection_id=cid)
    forge.revoke_connection(AGENT, cid, "rotate key", nonce, system["owner"].sign(payload).hex())
    after = signed_run(forge, key, "Pay x3-demo.near 1 USDC.", max_tokens=100)
    assert "connection_revoked" in after["reasons"]


def test_revoked_agent_stops_thinking(system):
    forge = system["forge"]
    key, _ = born_agent(system)
    assert signed_run(forge, key, "Pay x3-demo.near 1 USDC.")["verdict"] == "answered"
    nonce = secrets.token_hex(12)
    payload = forge.revoke_payload(AGENT, "retired", nonce)
    forge.revoke(AGENT, "retired", nonce, system["owner"].sign(payload).hex())
    out = signed_run(forge, key, "Pay x3-demo.near 1 USDC.")
    assert out["verdict"] == "refused" and "agent_revoked" in out["reasons"]


def test_raw_inference_and_no_secret_leak(system):
    forge = system["forge"]
    key, cid = born_agent(system, spend=False)
    nonce, exp = secrets.token_hex(12), int(forge.clock()) + 60
    msgs = [{"role": "user", "content": "Summarize the mandate rules in one line."}]
    body = {"model": MODEL, "messages": msgs, "max_tokens": 200}
    sig = key.sign(forge.infer_payload(AGENT, "X3/FORGE/INFER/1", nonce, exp, body)).hex()
    out = forge.infer(AGENT, MODEL, msgs, 200, nonce, exp, sig)
    assert out["verdict"] == "answered" and out["result"]["untrusted_output"] is True and out["result"]["text"]
    receipt = out["result"]["x3_mesh"]
    assert receipt["untrusted_output"] is True and receipt["worker_tier"] == "fleet" and receipt["worker_sig"]
    assert receipt["job_id"] == out["entry"]["mesh_job"]
    everything = json.dumps(forge.agent(AGENT)) + json.dumps(forge.export(AGENT)) + json.dumps(out)
    assert system["app_key"] not in everything


def test_cli_over_http_signs_locally(system, tmp_path, capsys):
    import base64
    from http.server import ThreadingHTTPServer as Srv

    from forge import agent_cli
    from forge.server import Api, make_handler

    forge = system["forge"]
    key, _ = born_agent(system)
    d = key.private_bytes(serialization.Encoding.Raw, serialization.PrivateFormat.Raw, serialization.NoEncryption())
    jwk = tmp_path / "vexa.agent-key.jwk.json"
    jwk.write_text(json.dumps({"kty": "OKP", "crv": "Ed25519", "d": base64.urlsafe_b64encode(d).rstrip(b"=").decode(),
                               "x": base64.urlsafe_b64encode(bytes.fromhex(raw(key.public_key()))).rstrip(b"=").decode()}))
    srv = Srv(("127.0.0.1", 0), make_handler(Api(forge), web_dist=tmp_path / "nodist"))
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    url = f"http://127.0.0.1:{srv.server_address[1]}"
    try:
        code = agent_cli.main(["--url", url, "--agent", AGENT, "--key", str(jwk), "--model", MODEL, "run",
                               "Pay x3-demo.near 10 USDC for invoice 77."])
        printed = capsys.readouterr().out
        assert code == 0, printed
        assert "result: allowed" in printed and "pass  mandate" in printed and "intent pay_invoice" in printed
        system["ollama"].recipient = "attacker.near"
        assert agent_cli.main(["--url", url, "--agent", AGENT, "--key", str(jwk), "--model", MODEL, "run",
                               "Pay x3-demo.near 10 USDC."]) == 1
        assert "FAIL  mandate" in capsys.readouterr().out
        other = tmp_path / "other.jwk.json"
        od = Ed25519PrivateKey.generate().private_bytes(serialization.Encoding.Raw, serialization.PrivateFormat.Raw,
                                                         serialization.NoEncryption())
        other.write_text(json.dumps({"kty": "OKP", "crv": "Ed25519", "d": base64.urlsafe_b64encode(od).decode()}))
        assert agent_cli.main(["--url", url, "--agent", AGENT, "--key", str(other), "--model", MODEL, "run", "x"]) == 1
        assert "gate: refused (signature)" in capsys.readouterr().out
    finally:
        srv.shutdown()
        srv.server_close()


def test_stranded_request_is_charged_in_full(system):
    """A host crash mid-request leaves a 'running' claim: it keeps its nonce and its whole max_tokens."""
    forge = system["forge"]
    key, _ = born_agent(system, daily_tokens=2000, max_tokens=1500)
    record = forge._load(AGENT)
    record.setdefault("infer_log", []).append({"nonce": "stranded-nonce-0001", "at": int(forge.clock()), "kind": "run",
                                               "model": MODEL, "max_tokens": 1500, "tokens": 0, "verdict": "running",
                                               "reasons": []})
    forge._save(record)
    out = signed_run(forge, key, "Pay x3-demo.near 1 USDC.", max_tokens=600)
    assert out["reasons"] == ["budget_exhausted"]
    assert signed_run(forge, key, "x", max_tokens=100, nonce="stranded-nonce-0001")["reasons"] == ["replay"]


def test_agent_without_spend_answers_in_plain_text(system):
    forge = system["forge"]
    key, _ = born_agent(system, spend=False)
    out = signed_run(forge, key, "What can you do?", max_tokens=200)
    assert out["verdict"] == "answered" and out["result"]["status"] == "answered"
    assert out["result"]["untrusted_output"] is True and "can't propose payments" in out["result"]["note"]
