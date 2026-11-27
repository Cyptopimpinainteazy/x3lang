"""X3 GPU contributor mesh: real coordinator and worker over HTTP, a fake Ollama (test only),
and the unchanged ``near_agent.OpenAICompatibleModel`` as the app."""

from __future__ import annotations

import json
import os
import sqlite3
import stat
import threading
import time
import urllib.error
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

import pytest

from gpu_mesh import gpu_probe, ledger
from gpu_mesh.coordinator import Config, Coordinator, serve
from gpu_mesh.protocol import (Identity, ProtocolError, canonical, normalize_chat_request, open_envelope, seal,
                               sha256_hex)
from gpu_mesh.store import Store, worker_result_message
from gpu_mesh.worker import OllamaClient, Worker, check_url

MODEL = "qwen3:8b"
DIGEST = "a" * 64


# --------------------------------------------------------------------------------------------
# Test-only stand-in for a contributor's local Ollama
# --------------------------------------------------------------------------------------------

class FakeOllama:
    """Ollama's /api/tags and streaming /api/chat (NDJSON).

    mode: honest (answers canaries, writes a program), liar (constant junk), broken (HTTP 500),
    slow (streams word by word with a pause), dies (streams a little, then breaks the stream)."""

    def __init__(self, mode: str = "honest", models=((MODEL, DIGEST),), pause_s: float = 0.0):
        self.mode = mode
        self.pause_s = pause_s
        self.models = list(models)
        self.calls = []
        fake = self

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *a):
                pass

            def _json(self, status, payload):
                body = json.dumps(payload).encode()
                self.send_response(status)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

            def do_GET(self):
                if self.path == "/api/tags":
                    self._json(200, {"models": [{"name": n, "model": n, "digest": d} for n, d in fake.models]})
                else:
                    self._json(404, {"error": "not found"})

            def do_POST(self):
                body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
                fake.calls.append(body)
                if self.path != "/api/chat" or fake.mode == "broken":
                    self._json(500, {"error": "CUDA out of memory"})
                    return
                last = body["messages"][-1]["content"]
                if fake.mode == "liar":
                    content = "I am definitely running a model."
                elif "verification code" in last:
                    content = last.rsplit(" ", 1)[-1]
                else:
                    content = "```x3\nintent pay { amount: 1 }\n```"
                self.send_response(200)
                self.send_header("Content-Type", "application/x-ndjson")
                self.end_headers()
                pieces = [content[i:i + 4] for i in range(0, len(content), 4)]
                for n, piece in enumerate(pieces):
                    if fake.mode == "dies" and n == 3:
                        self.wfile.flush()
                        self.connection.shutdown(2)
                        return
                    self.wfile.write(json.dumps({"model": body["model"], "message": {"role": "assistant",
                                                 "content": piece}, "done": False}).encode() + b"\n")
                    self.wfile.flush()
                    if fake.pause_s:
                        time.sleep(fake.pause_s)
                self.wfile.write(json.dumps({"model": body["model"], "message": {"role": "assistant", "content": ""},
                                             "done": True, "done_reason": "stop", "prompt_eval_count": 11,
                                             "eval_count": 7}).encode() + b"\n")

        self.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.url = f"http://127.0.0.1:{self.server.server_address[1]}"
        threading.Thread(target=self.server.serve_forever, daemon=True).start()

    def close(self):
        self.server.shutdown()
        self.server.server_close()


class Mesh:
    def __init__(self, tmp_path, **config):
        self.tmp = tmp_path
        self.store = Store(str(tmp_path / "mesh.db"))
        self.identity = Identity.generate()
        defaults = dict(poll_wait_s=1, pickup_timeout_s=3, job_timeout_s=10, heartbeat_ttl_s=30,
                        canary_interval_s=3600)
        defaults.update(config)
        self.coordinator = Coordinator(self.store, self.identity, Config(**defaults))
        self.server = serve(self.coordinator, "127.0.0.1", 0)
        self.url = f"http://127.0.0.1:{self.server.server_address[1]}"
        threading.Thread(target=self.server.serve_forever, daemon=True).start()
        self.workers, self.fakes = [], []

    def worker(self, label, *, fleet=True, mode="honest", models=((MODEL, DIGEST),), pause_s=0.0):
        fake = FakeOllama(mode, models, pause_s)
        self.fakes.append(fake)
        identity = Identity.generate()
        if fleet:
            self.store.enroll_fleet(identity.public_hex, label)
        w = Worker(identity, self.url, OllamaClient(fake.url), label=label, heartbeat_s=0.5,
                   gpus=lambda: [{"name": "RTX 3090", "vendor": "nvidia", "vram_mb": 24576}], idle_wait_s=0.2)
        w.start()
        self.workers.append(w)
        return w, fake

    def app(self, name="agents", **kw):
        kw.setdefault("allow_public", False)
        kw.setdefault("requests_per_minute", 600)
        kw.setdefault("daily_token_budget", 1_000_000)
        kw.setdefault("max_tokens_cap", 4096)
        return self.store.add_app_key(name, **kw)

    def close(self):
        for w in self.workers:
            w.stop()
        for w in self.workers:
            w.join(3)
        self.server.shutdown()
        self.server.server_close()
        for f in self.fakes:
            f.close()


@pytest.fixture
def mesh(tmp_path):
    m = Mesh(tmp_path)
    yield m
    m.close()


def post(url, body, key=None, raw=None):
    data = raw if raw is not None else json.dumps(body).encode()
    headers = {"Content-Type": "application/json"}
    if key:
        headers["Authorization"] = f"Bearer {key}"
    req = urllib.request.Request(url, data=data, headers=headers, method="POST")
    try:
        with urllib.request.urlopen(req, timeout=30) as resp:
            return resp.status, resp.read(), resp.headers
    except urllib.error.HTTPError as exc:
        return exc.code, exc.read(), exc.headers


def wait_for(predicate, timeout=8.0):
    end = time.time() + timeout
    while time.time() < end:
        if predicate():
            return True
        time.sleep(0.05)
    return False


CHAT = {"model": MODEL, "messages": [{"role": "user", "content": "pay 1 USDC"}], "temperature": 0}


# --------------------------------------------------------------------------------------------
# End to end
# --------------------------------------------------------------------------------------------

def test_fleet_gpu_serves_the_unchanged_near_agent(mesh):
    import near_agent

    w, fake = mesh.worker("x3gpu1")
    assert wait_for(lambda: w.status == "approved")
    key = mesh.app("agents")
    model = near_agent.OpenAICompatibleModel(base_url=f"{mesh.url}/v1", model=MODEL, api_key=key)
    out = model.complete("You write X3 programs.", "Pay 1 USDC to merchant.near")
    assert "intent pay" in out["text"]
    assert out["provider"] == "openai-compatible" and out["usage"]["completion_tokens"] == 7
    assert fake.calls[0]["messages"][0] == {"role": "system", "content": "You write X3 programs."}
    assert fake.calls[0]["options"]["temperature"] == 0 and fake.calls[0]["stream"] is True

    check = mesh.store.verify_receipts(mesh.identity.public_hex)
    assert check == {"ok": True, "receipts": 1, "head": check["head"]}
    doc = ledger.build(mesh.store, mesh.identity, since_ms=0)
    assert ledger.verify(doc)
    [entry] = doc["ledger"]["contributors"]
    assert entry["worker"] == w.identity.public_hex and entry["jobs"] == 1 and entry["by_app"] == {"agents": 1}


def test_response_carries_a_verifiable_receipt(mesh):
    w, _ = mesh.worker("x3gpu1")
    assert wait_for(lambda: w.status == "approved")
    status, raw, _ = post(f"{mesh.url}/v1/chat/completions", CHAT, mesh.app())
    assert status == 200
    data = json.loads(raw)
    receipt = data["x3_mesh"]
    assert receipt["untrusted_output"] is True and receipt["worker"] == w.identity.public_hex
    response = {"content": data["choices"][0]["message"]["content"], "finish_reason": "stop", "model": MODEL,
                "usage": {"prompt_tokens": 11, "completion_tokens": 7}}
    assert sha256_hex(canonical(response)) == receipt["response_sha256"]
    from gpu_mesh.protocol import verify_signature
    message = worker_result_message({**receipt, "job_id": receipt["job_id"]})
    assert verify_signature(receipt["worker"], message.encode(), receipt["worker_sig"])
    assert verify_signature(receipt["coordinator"], receipt["receipt_hash"].encode(), receipt["coordinator_sig"])


def read_sse(url, body, key):
    """POST and read server-sent events as they arrive. Returns [(seconds_since_start, payload)]."""
    import http.client
    from urllib.parse import urlparse

    u = urlparse(url)
    conn = http.client.HTTPConnection(u.hostname, u.port, timeout=30)
    start = time.monotonic()
    conn.request("POST", u.path, json.dumps(body), {"Content-Type": "application/json",
                                                     "Authorization": f"Bearer {key}"})
    resp = conn.getresponse()
    assert resp.status == 200 and resp.getheader("Content-Type") == "text/event-stream", resp.status
    events = []
    while True:
        line = resp.fp.readline()
        if not line:
            break
        line = line.decode().strip()
        if line.startswith("data: "):
            data = line[len("data: "):]
            events.append((time.monotonic() - start, data if data == "[DONE]" else json.loads(data)))
            if data == "[DONE]":
                break
    conn.close()
    return events


def test_streaming_for_ide_chat_clients(mesh):
    w, _ = mesh.worker("x3gpu1")
    assert wait_for(lambda: w.status == "approved")
    body = {**CHAT, "stream": True, "stream_options": {"include_usage": True}}
    events = [e for _, e in read_sse(f"{mesh.url}/v1/chat/completions", body, mesh.app("ide"))]
    assert events[-1] == "[DONE]"
    chunks = events[:-1]
    deltas = [c["choices"][0]["delta"] for c in chunks if c.get("choices")]
    assert deltas[0]["role"] == "assistant"
    text = "".join(d.get("content", "") for d in deltas)
    assert text == "```x3\nintent pay { amount: 1 }\n```"
    [final] = [c for c in chunks if c.get("choices") and c["choices"][0]["finish_reason"]]
    assert final["choices"][0]["finish_reason"] == "stop" and final["x3_mesh"]["app"] == "ide"
    response = {"content": text, "finish_reason": "stop", "model": MODEL,
                "usage": {"prompt_tokens": 11, "completion_tokens": 7}}
    assert sha256_hex(canonical(response)) == final["x3_mesh"]["response_sha256"], "receipt covers streamed text"
    assert chunks[-1]["usage"]["total_tokens"] == 18


def test_tokens_arrive_while_the_gpu_is_still_generating(mesh):
    w, _ = mesh.worker("x3gpu-slow", pause_s=0.15)
    assert wait_for(lambda: w.status == "approved")
    events = read_sse(f"{mesh.url}/v1/chat/completions", {**CHAT, "stream": True}, mesh.app("ide"))
    content_times = [t for t, e in events if e != "[DONE]" and e.get("choices")
                     and e["choices"][0]["delta"].get("content")]
    final_time = next(t for t, e in events if e != "[DONE]" and e.get("x3_mesh"))
    assert len(content_times) >= 3, "the answer should arrive in several pieces"
    assert final_time - content_times[0] >= 0.5, "the first words must arrive well before the end"


def test_mid_stream_failure_is_reported_not_retried(mesh):
    dying, dying_fake = mesh.worker("x3gpu-dies", mode="dies", pause_s=0.15)
    good, good_fake = mesh.worker("x3gpu-ok")
    assert wait_for(lambda: dying.status == "approved" and good.status == "approved")
    db = sqlite3.connect(mesh.store.path)
    db.execute("UPDATE workers SET jobs_ok = 1, jobs_failed = 1 WHERE pubkey = ?", (good.identity.public_hex,))
    db.commit()
    db.close()
    events = [e for _, e in read_sse(f"{mesh.url}/v1/chat/completions", {**CHAT, "stream": True},
                                     mesh.app("ide"))]
    assert events[-1] == "[DONE]"
    assert any(e.get("choices") and e["choices"][0]["delta"].get("content") for e in events[:-1])
    assert events[-2]["error"]["code"] == "worker_failed"
    assert good_fake.calls == [], "text already reached the client, so the job must not silently restart elsewhere"


def test_stream_before_any_output_is_retried_elsewhere(mesh):
    bad, _ = mesh.worker("x3gpu-broken", mode="broken")
    good, _ = mesh.worker("x3gpu-ok")
    assert wait_for(lambda: bad.status == "approved" and good.status == "approved")
    db = sqlite3.connect(mesh.store.path)
    db.execute("UPDATE workers SET jobs_ok = 1, jobs_failed = 1 WHERE pubkey = ?", (good.identity.public_hex,))
    db.commit()
    db.close()
    events = [e for _, e in read_sse(f"{mesh.url}/v1/chat/completions", {**CHAT, "stream": True},
                                     mesh.app("ide"))]
    final = next(e for e in events if e != "[DONE]" and e.get("x3_mesh"))
    assert final["x3_mesh"]["worker"] == good.identity.public_hex


def test_models_endpoint_lists_what_contributors_serve(mesh):
    w, _ = mesh.worker("x3gpu1", models=((MODEL, DIGEST), ("llama3.1:8b", "b" * 64)))
    assert wait_for(lambda: w.status == "approved")
    req = urllib.request.Request(f"{mesh.url}/v1/models", headers={"Authorization": f"Bearer {mesh.app()}"})
    with urllib.request.urlopen(req, timeout=10) as resp:
        data = json.loads(resp.read())
    assert [m["id"] for m in data["data"]] == ["llama3.1:8b", MODEL]


def test_public_contributor_needs_approval_and_a_passed_canary_and_an_allow_public_app(tmp_path):
    mesh = Mesh(tmp_path, canary_interval_s=3600)
    try:
        w, fake = mesh.worker("stranger", fleet=False)
        assert wait_for(lambda: w.status == "pending")
        fleet_only = mesh.app("ide")
        status, raw, _ = post(f"{mesh.url}/v1/chat/completions", CHAT, fleet_only)
        assert status == 503 and json.loads(raw)["error"]["code"] == "no_capacity"

        mesh.store.set_status(w.identity.public_hex, "approved", "reviewed by owner")
        assert wait_for(lambda: mesh.store.worker(w.identity.public_hex)["canary_pass"] == 1)
        assert "verification code" in fake.calls[0]["messages"][-1]["content"]

        status, _, _ = post(f"{mesh.url}/v1/chat/completions", CHAT, fleet_only)
        assert status == 503, "a fleet-only app must never reach a public contributor"
        status, raw, _ = post(f"{mesh.url}/v1/chat/completions", CHAT, mesh.app("site", allow_public=True))
        assert status == 200 and json.loads(raw)["x3_mesh"]["worker_tier"] == "public"
        assert mesh.store.verify_receipts(mesh.identity.public_hex)["ok"]
    finally:
        mesh.close()


def test_contributor_that_does_not_run_the_model_is_suspended(tmp_path):
    mesh = Mesh(tmp_path, canary_interval_s=0)
    try:
        w, _ = mesh.worker("faker", fleet=False, mode="liar")
        assert wait_for(lambda: w.status == "pending")
        mesh.store.set_status(w.identity.public_hex, "approved", "reviewed")
        assert wait_for(lambda: mesh.store.worker(w.identity.public_hex)["status"] == "suspended")
        info = mesh.store.worker(w.identity.public_hex)
        assert info["canary_fail"] >= 2 and info["canary_pass"] == 0 and "canary" in info["status_reason"]
        status, _, _ = post(f"{mesh.url}/v1/chat/completions", CHAT, mesh.app("site", allow_public=True))
        assert status == 503
    finally:
        mesh.close()


def test_failed_gpu_is_retried_on_another_contributor(mesh):
    bad, bad_fake = mesh.worker("x3gpu-broken", mode="broken")
    good, _ = mesh.worker("x3gpu-ok")
    assert wait_for(lambda: bad.status == "approved" and good.status == "approved")
    # Give the healthy machine a worse record so the router deterministically tries the broken one first.
    db = sqlite3.connect(mesh.store.path)
    db.execute("UPDATE workers SET jobs_ok = 1, jobs_failed = 1 WHERE pubkey = ?", (good.identity.public_hex,))
    db.commit()
    db.close()
    key = mesh.app()
    for _ in range(3):
        status, raw, _ = post(f"{mesh.url}/v1/chat/completions", CHAT, key)
        assert status == 200
        assert json.loads(raw)["x3_mesh"]["worker"] == good.identity.public_hex
    assert mesh.store.worker(bad.identity.public_hex)["jobs_failed"] == 1
    assert len(bad_fake.calls) == 1, "after one failure the router stops preferring the broken machine"


def test_repeated_failures_suspend_even_a_fleet_machine(tmp_path):
    mesh = Mesh(tmp_path, job_failures_to_suspend=2)
    try:
        bad, _ = mesh.worker("x3gpu-broken", mode="broken")
        assert wait_for(lambda: bad.status == "approved")
        key = mesh.app()
        for _ in range(2):
            status, _, _ = post(f"{mesh.url}/v1/chat/completions", CHAT, key)
            assert status == 502
        info = mesh.store.worker(bad.identity.public_hex)
        assert info["status"] == "suspended" and "failed jobs in a row" in info["status_reason"]
        assert post(f"{mesh.url}/v1/chat/completions", CHAT, key)[0] == 503
    finally:
        mesh.close()


# --------------------------------------------------------------------------------------------
# App-side limits
# --------------------------------------------------------------------------------------------

def test_app_key_and_request_checks(mesh):
    url = f"{mesh.url}/v1/chat/completions"
    assert post(url, CHAT)[0] == 401
    assert post(url, CHAT, "x3m_not-a-real-key")[0] == 401
    key = mesh.app()
    status, raw, _ = post(url, {**CHAT, "tools": [{"type": "function"}]}, key)
    assert status == 400 and json.loads(raw)["error"]["code"] == "unsupported_parameter"
    assert post(url, {**CHAT, "stream": "yes"}, key)[0] == 400
    assert post(url, {**CHAT, "messages": [{"role": "tool", "content": "x"}]}, key)[0] == 400
    assert post(url, None, key, raw=b"x" * (300 * 1024))[0] == 413
    mesh.store.revoke_app_key("agents")
    assert post(url, CHAT, key)[0] == 401


def test_rate_limit_and_daily_budget(mesh):
    w, _ = mesh.worker("x3gpu1")
    assert wait_for(lambda: w.status == "approved")
    url = f"{mesh.url}/v1/chat/completions"
    limited = mesh.app("site", requests_per_minute=2)
    assert [post(url, CHAT, limited)[0] for _ in range(3)] == [200, 200, 429]
    budget = mesh.app("tiny", daily_token_budget=1010)
    assert post(url, {**CHAT, "max_tokens": 1000}, budget)[0] == 200
    status, raw, _ = post(url, {**CHAT, "max_tokens": 1000}, budget)
    assert status == 429 and json.loads(raw)["error"]["code"] == "budget_exhausted"


def test_max_tokens_is_capped_per_app(mesh):
    w, fake = mesh.worker("x3gpu1")
    assert wait_for(lambda: w.status == "approved")
    key = mesh.app("ide", max_tokens_cap=256)
    assert post(f"{mesh.url}/v1/chat/completions", {**CHAT, "max_tokens": 4000}, key)[0] == 200
    assert fake.calls[-1]["options"]["num_predict"] == 256


# --------------------------------------------------------------------------------------------
# Contributor-side attacks, against the coordinator directly
# --------------------------------------------------------------------------------------------

class Direct:
    """A registered fleet contributor driven by hand, so tests can forge and replay messages."""

    def __init__(self, coord: Coordinator, label="x3gpu-direct", fleet=True):
        self.coord, self.id = coord, Identity.generate()
        if fleet:
            coord.store.enroll_fleet(self.id.public_hex, label)
        challenge = coord.store.new_challenge()["challenge"]
        coord.register(seal(self.id, "register", {"challenge": challenge, "label": label, "gpus": [],
                                                  "models": [{"name": MODEL, "digest": DIGEST}]}))

    def poll(self):
        return self.coord.poll(seal(self.id, "poll", {}), wait_s=5)

    def result_payload(self, job, content="ok", identity=None):
        response = {"content": content, "finish_reason": "stop", "model": MODEL,
                    "usage": {"prompt_tokens": 1, "completion_tokens": 1}}
        response_sha = sha256_hex(canonical(response))
        signer = identity or self.id
        sig = signer.sign(worker_result_message({"job_id": job["job_id"], "request_sha256": job["request_sha256"],
                                                 "response_sha256": response_sha, "model": MODEL,
                                                 "model_digest": DIGEST}).encode())
        return {"job_id": job["job_id"], "request_sha256": job["request_sha256"], "response": response,
                "response_sha256": response_sha, "model_digest": DIGEST, "result_sig": sig, "started_ms": 1,
                "finished_ms": 2}


@pytest.fixture
def coord(tmp_path):
    store = Store(str(tmp_path / "direct.db"))
    c = Coordinator(store, Identity.generate(), Config(poll_wait_s=1, pickup_timeout_s=5, job_timeout_s=8))
    yield c
    store.close()


def start_completion(coord, key_name="agents"):
    key = coord.store.add_app_key(key_name, allow_public=False, requests_per_minute=100, daily_token_budget=10 ** 6,
                                  max_tokens_cap=1024)
    app = coord.authenticate(f"Bearer {key}")
    out = {}

    def run():
        try:
            out["response"], _ = coord.complete(app, CHAT)
        except ProtocolError as exc:
            out["error"] = exc

    thread = threading.Thread(target=run, daemon=True)
    thread.start()
    return thread, out


def test_replayed_and_forged_envelopes_are_refused(coord):
    a = Direct(coord)
    envelope = seal(a.id, "poll", {})
    assert coord.poll(envelope, wait_s=0) is None
    with pytest.raises(ProtocolError) as exc:
        coord.poll(envelope, wait_s=0)
    assert exc.value.code == "replay"

    forged = seal(a.id, "heartbeat", {"gpus": [], "models": [{"name": MODEL, "digest": DIGEST}]})
    forged["payload"]["models"][0]["name"] = "llama3.1:8b"
    with pytest.raises(ProtocolError) as exc:
        coord.heartbeat(forged)
    assert exc.value.code == "bad_signature"

    with pytest.raises(ProtocolError) as exc:
        coord.poll(seal(a.id, "heartbeat", {}), wait_s=0)
    assert "expected a 'poll' envelope" in str(exc.value)

    with pytest.raises(ProtocolError) as exc:
        open_envelope(seal(a.id, "poll", {}, ts=int(time.time()) - 3600), "poll")
    assert exc.value.code == "stale"


def test_registration_challenge_is_single_use(coord):
    ident = Identity.generate()
    challenge = coord.store.new_challenge()["challenge"]
    payload = {"challenge": challenge, "label": "someone", "models": [{"name": MODEL, "digest": DIGEST}]}
    assert coord.register(seal(ident, "register", payload))["status"] == "pending"
    with pytest.raises(ProtocolError) as exc:
        coord.register(seal(Identity.generate(), "register", payload))
    assert exc.value.code == "bad_challenge"


def test_pending_and_revoked_contributors_get_no_work(coord):
    stranger = Direct(coord, "stranger", fleet=False)
    with pytest.raises(ProtocolError) as exc:
        stranger.poll()
    assert exc.value.status == 403 and exc.value.code == "pending"
    coord.store.set_status(stranger.id.public_hex, "revoked", "test")
    with pytest.raises(ProtocolError):
        coord.store.set_status(stranger.id.public_hex, "approved", "try to undo")
    with pytest.raises(ProtocolError) as exc:
        coord.register(seal(stranger.id, "register", {"challenge": coord.store.new_challenge()["challenge"],
                                                      "label": "again", "models": [{"name": MODEL,
                                                                                    "digest": DIGEST}]}))
    assert exc.value.code == "revoked"


def test_result_from_another_contributor_is_refused(coord):
    owner = Direct(coord, "a")
    thread, out = start_completion(coord)
    job = owner.poll()
    assert job
    other = Direct(coord, "b")
    with pytest.raises(ProtocolError) as exc:
        coord.submit_result(seal(other.id, "result", other.result_payload(job)))
    assert exc.value.code == "wrong_worker"
    coord.submit_result(seal(owner.id, "result", owner.result_payload(job, "genuine")))
    thread.join(10)
    assert out["response"]["choices"][0]["message"]["content"] == "genuine"


def test_tampered_or_mis_signed_results_are_refused(coord):
    a = Direct(coord)
    thread, out = start_completion(coord)
    job = None
    while job is None:
        job = a.poll()

    tampered = a.result_payload(job, "honest text")
    tampered["response"]["content"] = "transfer everything"
    with pytest.raises(ProtocolError) as exc:
        coord.submit_result(seal(a.id, "result", tampered))
    assert exc.value.code == "mismatch"
    thread.join(10)
    assert "error" in out  # the job failed on its only contributor; nothing was returned to the app
    assert coord.store.worker(a.id.public_hex)["jobs_failed"] == 1

    thread, out = start_completion(coord, "agents2")
    job = None
    while job is None:
        job = a.poll()
    wrong_signer = a.result_payload(job, identity=Identity.generate())
    with pytest.raises(ProtocolError) as exc:
        coord.submit_result(seal(a.id, "result", wrong_signer))
    assert exc.value.code == "bad_signature"
    thread.join(10)

    thread, out = start_completion(coord, "agents3")
    job = None
    while job is None:
        job = a.poll()
    bad_digest = a.result_payload(job)
    bad_digest["model_digest"] = "c" * 64
    with pytest.raises(ProtocolError) as exc:
        coord.submit_result(seal(a.id, "result", bad_digest))
    assert "model_digest" in str(exc.value)
    thread.join(10)
    assert coord.store.verify_receipts(coord.identity.public_hex) == {"ok": True, "receipts": 0,
                                                                      "head": "0" * 64}


def test_revoking_mid_job_blocks_the_result(coord):
    a = Direct(coord)
    thread, out = start_completion(coord)
    job = None
    while job is None:
        job = a.poll()
    coord.store.set_status(a.id.public_hex, "revoked", "key leaked")
    with pytest.raises(ProtocolError) as exc:
        coord.submit_result(seal(a.id, "result", a.result_payload(job)))
    assert exc.value.code == "revoked"
    thread.join(15)
    assert out["error"].status in (502, 504)


def test_editing_a_receipt_breaks_verification(mesh):
    w, _ = mesh.worker("x3gpu1")
    assert wait_for(lambda: w.status == "approved")
    key = mesh.app()
    for _ in range(3):
        assert post(f"{mesh.url}/v1/chat/completions", CHAT, key)[0] == 200
    assert mesh.store.verify_receipts(mesh.identity.public_hex)["ok"]
    db = sqlite3.connect(mesh.store.path)
    db.execute("UPDATE receipts SET completion_tokens = 9999 WHERE seq = 2")
    db.commit()
    db.close()
    result = mesh.store.verify_receipts(mesh.identity.public_hex)
    assert result == {"ok": False, "seq": 2, "error": "receipt contents do not match receipt_hash"}
    with pytest.raises(RuntimeError):
        ledger.build(mesh.store, mesh.identity, since_ms=0)


def test_ledger_signature_detects_edits(mesh):
    doc = ledger.build(mesh.store, mesh.identity, since_ms=0)
    assert ledger.verify(doc)
    doc["ledger"]["contributors"].append({"worker": "f" * 64, "jobs": 10 ** 6})
    assert not ledger.verify(doc)


# --------------------------------------------------------------------------------------------
# Units
# --------------------------------------------------------------------------------------------

def test_key_file_is_private_and_never_overwritten(tmp_path):
    path = str(tmp_path / "keys" / "worker.key")
    ident = Identity.generate()
    ident.save(path)
    assert stat.S_IMODE(os.stat(path).st_mode) == 0o600
    assert Identity.load(path).public_hex == ident.public_hex
    with pytest.raises(FileExistsError):
        Identity.generate().save(path)


def test_canonical_request_refuses_what_it_cannot_honour():
    request, stream = normalize_chat_request({**CHAT, "max_tokens": 99999, "stream": True}, max_tokens_cap=512)
    assert stream and request["max_tokens"] == 512 and request["temperature_milli"] == 0
    for bad in ({**CHAT, "functions": []}, {**CHAT, "n": 2}, {**CHAT, "temperature": 3},
                {**CHAT, "messages": [{"role": "user", "content": [{"type": "image_url"}]}]},
                {**CHAT, "model": "bad model name"}, {**CHAT, "surprise": 1}):
        with pytest.raises(ProtocolError):
            normalize_chat_request(bad)
    with pytest.raises(ProtocolError):
        canonical({"x": 0.5})


def test_worker_refuses_plain_http_to_remote_hosts():
    assert check_url("https://gpu.x3star.net/", allow_http=False, what="c") == "https://gpu.x3star.net"
    assert check_url("http://127.0.0.1:8790", allow_http=False, what="c")
    with pytest.raises(ValueError):
        check_url("http://gpu.x3star.net", allow_http=False, what="c")
    assert check_url("http://10.0.0.5:8790", allow_http=True, what="c")


def test_gpu_probe_parsers():
    nvidia = "NVIDIA GeForce RTX 3090, 24576, 1024, 37, 61\nNVIDIA GeForce GTX 1070, 8192, 0, 0, 40\n"
    assert gpu_probe.parse_nvidia_smi(nvidia) == [
        {"name": "NVIDIA GeForce RTX 3090", "vendor": "nvidia", "vram_mb": 24576, "mem_used_mb": 1024,
         "util_pct": 37, "temp_c": 61},
        {"name": "NVIDIA GeForce GTX 1070", "vendor": "nvidia", "vram_mb": 8192, "mem_used_mb": 0, "util_pct": 0,
         "temp_c": 40}]
    rocm = json.dumps({"card0": {"Card series": "Radeon Instinct MI50", "VRAM Total Memory (B)": "34342961152",
                                 "VRAM Total Used Memory (B)": "10485760"}})
    assert gpu_probe.parse_rocm_smi(rocm) == [{"name": "Radeon Instinct MI50", "vendor": "amd", "vram_mb": 32752,
                                               "mem_used_mb": 10}]
    assert gpu_probe.parse_nvidia_smi("") == [] and gpu_probe.parse_rocm_smi("not json") == []


# --------------------------------------------------------------------------------------------
# Resource limits for unauthenticated or unknown parties
# --------------------------------------------------------------------------------------------

def test_strangers_cannot_grow_the_nonce_table(coord):
    for _ in range(20):
        with pytest.raises(ProtocolError) as exc:
            coord.poll(seal(Identity.generate(), "poll", {}), wait_s=0)
        assert exc.value.code == "not_registered"
    db = sqlite3.connect(coord.store.path)
    assert db.execute("SELECT COUNT(*) FROM nonces").fetchone()[0] == 0
    db.close()


def test_pending_registrations_and_open_challenges_are_capped(tmp_path):
    store = Store(str(tmp_path / "caps.db"))
    c = Coordinator(store, Identity.generate(), Config(max_pending=2, max_open_challenges=3))
    models = [{"name": MODEL, "digest": DIGEST}]
    for i in range(2):
        ch = store.new_challenge(max_open=3)["challenge"]
        assert c.register(seal(Identity.generate(), "register", {"challenge": ch, "label": f"p{i}",
                                                                 "models": models}))["status"] == "pending"
    ch = store.new_challenge(max_open=3)["challenge"]
    with pytest.raises(ProtocolError) as exc:
        c.register(seal(Identity.generate(), "register", {"challenge": ch, "label": "p3", "models": models}))
    assert exc.value.code == "pending_full"
    fleet = Identity.generate()
    store.enroll_fleet(fleet.public_hex, "x3gpu9")
    ch = store.new_challenge(max_open=3)["challenge"]
    assert c.register(seal(fleet, "register", {"challenge": ch, "label": "x3gpu9", "models": models}))["status"] \
        == "approved", "your own enrolled machines are never blocked by the public queue"
    for _ in range(3):
        store.new_challenge(max_open=3)
    with pytest.raises(ProtocolError) as exc:
        store.new_challenge(max_open=3)
    assert exc.value.status == 429
    store.close()


def test_a_canary_that_never_returns_is_failed_and_frees_the_slot(tmp_path):
    store = Store(str(tmp_path / "lost.db"))
    c = Coordinator(store, Identity.generate(), Config(canary_interval_s=0, job_timeout_s=1, result_grace_s=0,
                                                       canary_failures_to_suspend=5))
    a = Direct(c, "stranger", fleet=False)
    store.set_status(a.id.public_hex, "approved", "reviewed")
    first = a.poll()
    assert first and "verification code" in first["request"]["messages"][-1]["content"]
    assert c._running[a.id.public_hex] == 1
    time.sleep(1.2)
    second = a.poll()  # sweeps the lost canary, then hands out a fresh one
    assert second and second["job_id"] != first["job_id"]
    info = store.worker(a.id.public_hex)
    assert info["canary_fail"] == 1 and c._running[a.id.public_hex] == 1
    store.close()


def start_stream(coord, key_name="ide"):
    key = coord.store.add_app_key(key_name, allow_public=False, requests_per_minute=100, daily_token_budget=10 ** 6,
                                  max_tokens_cap=1024)
    app = coord.authenticate(f"Bearer {key}")
    events = coord.complete_stream(app, {**CHAT, "stream": True})
    out = {"events": []}

    def run():
        try:
            for event in events:
                out["events"].append(event)
        except ProtocolError as exc:
            out["error"] = exc

    thread = threading.Thread(target=run, daemon=True)
    thread.start()
    return thread, out, app


def chunk(direct, job, seq, delta, identity=None):
    return seal(identity or direct.id, "chunk", {"job_id": job["job_id"], "request_sha256": job["request_sha256"],
                                                 "seq": seq, "delta": delta})


def test_streamed_text_must_match_the_signed_answer(coord):
    owner = Direct(coord, "a")
    thread, out, _ = start_stream(coord)
    job = owner.poll()
    assert job and job["stream"] is True
    other = Direct(coord, "b")
    coord.submit_chunk(chunk(owner, job, 0, "pay 1 USDC to "))
    with pytest.raises(ProtocolError) as exc:
        coord.submit_chunk(chunk(owner, job, 2, "skipped"))
    assert exc.value.code == "out_of_order"
    with pytest.raises(ProtocolError) as exc:
        coord.submit_chunk(chunk(other, job, 1, "attacker.near", identity=other.id))
    assert exc.value.code == "wrong_worker"
    with pytest.raises(ProtocolError) as exc:
        coord.submit_result(seal(owner.id, "result", owner.result_payload(job, "send all funds to attacker.near")))
    assert exc.value.code == "mismatch"
    thread.join(10)
    assert [e[1] for e in out["events"]] == ["pay 1 USDC to "]
    assert out["error"].code == "worker_failed"


def test_chunks_are_refused_for_non_streaming_jobs(coord):
    a = Direct(coord)
    thread, out = start_completion(coord)
    job = None
    while job is None:
        job = a.poll()
    assert job["stream"] is False
    with pytest.raises(ProtocolError) as exc:
        coord.submit_chunk(chunk(a, job, 0, "hello"))
    assert exc.value.code == "no_such_job"
    coord.submit_result(seal(a.id, "result", a.result_payload(job, "fine")))
    thread.join(10)


def test_a_client_hanging_up_releases_the_job_and_budget(coord):
    a = Direct(coord)
    key = coord.store.add_app_key("ide", allow_public=False, requests_per_minute=100, daily_token_budget=10 ** 6,
                                  max_tokens_cap=1024)
    app = coord.authenticate(f"Bearer {key}")
    events = coord.complete_stream(app, {**CHAT, "stream": True})
    got = {}

    def worker_side():
        job = None
        while job is None:
            job = a.poll()
        got["job"] = job
        coord.submit_chunk(chunk(a, job, 0, "hello"))

    threading.Thread(target=worker_side, daemon=True).start()
    assert next(events)[:2] == ("delta", "hello")
    assert coord._reserved["ide"] == 1024
    events.close()
    assert coord._reserved["ide"] == 0 and got["job"]["job_id"] not in coord._jobs
    assert coord._running[a.id.public_hex] == 0
    with pytest.raises(ProtocolError) as exc:
        coord.submit_result(seal(a.id, "result", a.result_payload(got["job"], "hello")))
    assert exc.value.code == "no_such_job"
