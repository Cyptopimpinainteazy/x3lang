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
    """mode: honest (answers canaries, echoes a program), liar (constant junk), broken (HTTP 500)."""

    def __init__(self, mode: str = "honest", models=((MODEL, DIGEST),)):
        self.mode = mode
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
                if fake.mode == "broken":
                    self._json(500, {"error": {"message": "CUDA out of memory"}})
                    return
                last = body["messages"][-1]["content"]
                if fake.mode == "liar":
                    content = "I am definitely running a model."
                elif "verification code" in last:
                    content = last.rsplit(" ", 1)[-1]
                else:
                    content = "```x3\nintent pay { amount: 1 }\n```"
                self._json(200, {"id": "chatcmpl-1", "model": body["model"],
                                 "choices": [{"index": 0, "message": {"role": "assistant", "content": content},
                                              "finish_reason": "stop"}],
                                 "usage": {"prompt_tokens": 11, "completion_tokens": 7, "total_tokens": 18}})

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

    def worker(self, label, *, fleet=True, mode="honest", models=((MODEL, DIGEST),)):
        fake = FakeOllama(mode, models)
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
    assert fake.calls[0]["temperature"] == 0

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


def test_streaming_for_ide_chat_clients(mesh):
    w, _ = mesh.worker("x3gpu1")
    assert wait_for(lambda: w.status == "approved")
    body = {**CHAT, "stream": True, "stream_options": {"include_usage": True}}
    status, raw, headers = post(f"{mesh.url}/v1/chat/completions", body, mesh.app("ide"))
    assert status == 200 and headers["Content-Type"] == "text/event-stream"
    events = [line[len("data: "):] for line in raw.decode().split("\n\n") if line.startswith("data: ")]
    assert events[-1] == "[DONE]"
    chunks = [json.loads(e) for e in events[:-1]]
    assert "".join(c["choices"][0]["delta"].get("content", "") for c in chunks if c["choices"]).startswith("```x3")
    assert chunks[1]["choices"][0]["finish_reason"] == "stop" and chunks[1]["x3_mesh"]["app"] == "ide"
    assert chunks[2]["usage"]["total_tokens"] == 18


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
    assert fake.calls[-1]["max_tokens"] == 256


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
    a, b = Direct(coord, "a"), Direct(coord, "b")
    thread, out = start_completion(coord)
    job = None
    for _ in range(20):
        job = a.poll() or b.poll()
        if job:
            break
    assert job
    owner = a if coord._jobs[job["job_id"]].worker == a.id.public_hex else b
    other = b if owner is a else a
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
