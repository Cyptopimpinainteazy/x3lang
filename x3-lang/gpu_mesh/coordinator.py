"""The coordinator: an OpenAI-compatible endpoint backed by contributed GPUs.

Apps (the CLI agents, the dotx3.net agents, the Super IDE) call
``POST /v1/chat/completions`` with their own app key, exactly as they would call Ollama or any
OpenAI-style server. The coordinator picks an approved contributor that serves the model,
hands it the job when the contributor long-polls, checks the signed result, appends a
hash-chained receipt, and answers the app. Nothing on a contributor's machine has to accept
inbound connections.

Trust model:

* Contributors are untrusted. Every message they send is signed (``protocol.open_envelope``),
  replay-protected, and checked against the job it claims to answer.
* Model output is untrusted. Apps get an ``x3_mesh`` receipt that says which contributor
  produced the text and that it is ``untrusted_output``; the X3 compiler, the owner's mandate
  and Guard still decide what may happen.
* Public contributors see the prompts they run. An app key only reaches public contributors
  when it was created with ``--allow-public``; otherwise its requests run on fleet machines
  only. A public contributor gets no real traffic until it has passed a canary job.
* Canary jobs catch a contributor that does not actually run the model. They cannot catch a
  contributor that runs the model and alters some answers; that is what the deterministic
  checks after the model are for.
"""

from __future__ import annotations

import json
import logging
import random
import secrets
import threading
import time
from collections import deque
from dataclasses import dataclass, field
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Any, Deque, Dict, List, Optional, Tuple
from urllib.parse import urlparse

from . import __version__
from .protocol import (MAX_BODY_BYTES, MAX_CLOCK_SKEW_S, PROTOCOL, Identity, ProtocolError, canonical,
                       normalize_chat_request, now_ms, now_s, open_envelope, sha256_hex, validate_gpus,
                       validate_label, validate_models, validate_payout_account, validate_response,
                       verify_signature)
from .store import Store, worker_result_message

log = logging.getLogger("x3.gpu_mesh.coordinator")

CANARY_APP = "_canary"


@dataclass
class Config:
    heartbeat_ttl_s: int = 90
    poll_wait_s: int = 25
    pickup_timeout_s: int = 15
    job_timeout_s: int = 110          # below the agent's 120 s client timeout
    result_grace_s: int = 5
    max_attempts: int = 3
    canary_interval_s: int = 900
    canary_failures_to_suspend: int = 2
    job_failures_to_suspend: int = 3
    nonce_ttl_s: int = 2 * MAX_CLOCK_SKEW_S + 60
    max_pending: int = 500
    max_open_challenges: int = 2000


@dataclass
class Job:
    job_id: str
    kind: str                       # "job" or "canary" (contributors are never told which)
    app: str
    request: Dict[str, Any]
    request_sha256: str
    deadline_ms: int
    canary_code: Optional[str] = None
    worker: Optional[str] = None
    state: str = "queued"           # queued -> running -> done | failed | expired
    started_ms: int = 0
    tried: List[str] = field(default_factory=list)
    result: Optional[Dict[str, Any]] = None
    error: Optional[str] = None

    @property
    def model(self) -> str:
        return self.request["model"]

    def handout(self) -> Dict[str, Any]:
        return {"job_id": self.job_id, "request": self.request, "request_sha256": self.request_sha256,
                "deadline_ms": self.deadline_ms}


class Coordinator:
    def __init__(self, store: Store, identity: Identity, config: Optional[Config] = None):
        self.store = store
        self.identity = identity
        self.config = config or Config()
        self._cv = threading.Condition()
        self._queues: Dict[str, Deque[Job]] = {}
        self._jobs: Dict[str, Job] = {}
        self._running: Dict[str, int] = {}
        self._rate: Dict[str, Deque[float]] = {}
        self._reserved: Dict[str, int] = {}
        store.meta_set("coordinator_pubkey", identity.public_hex)

    # ---------------------------------------------------------------- contributor side

    def _open(self, envelope: Any, kind: str) -> Tuple[str, Dict[str, Any]]:
        """Verify an envelope and record its nonce.

        Nonces are only stored for keys the coordinator already knows, so a stranger signing
        envelopes with throwaway keys cannot grow the database. Registration is the exception;
        it is gated by a single-use challenge instead (see ``register``).
        """
        worker, nonce, _ts, payload = open_envelope(envelope, kind)
        if kind != "register":
            info = self.store.worker(worker)
            if info is None:
                raise ProtocolError("unknown contributor; register first", status=403, code="not_registered")
            if info["status"] == "revoked":
                raise ProtocolError(f"contributor is revoked: {info['status_reason']}", status=403, code="revoked")
        self.store.consume_nonce(worker, nonce, self.config.nonce_ttl_s)
        return worker, payload

    def _require_approved(self, worker: str) -> Dict[str, Any]:
        info = self.store.worker(worker)
        if info is None:
            raise ProtocolError("unknown contributor; register first", status=403, code="not_registered")
        if info["status"] != "approved":
            raise ProtocolError(f"contributor is {info['status']}: {info['status_reason']}", status=403,
                                code=info["status"])
        return info

    def register(self, envelope: Any) -> Dict[str, Any]:
        worker, nonce, _ts, payload = open_envelope(envelope, "register")
        allowed = {"challenge", "label", "payout_account", "gpus", "models", "max_concurrency", "version"}
        if set(payload) - allowed:
            raise ProtocolError("unexpected register fields: " + ", ".join(sorted(set(payload) - allowed)))
        label = validate_label(payload.get("label"))
        payout = validate_payout_account(payload.get("payout_account"))
        gpus = validate_gpus(payload.get("gpus", []))
        models = validate_models(payload.get("models"))
        concurrency = payload.get("max_concurrency", 1)
        if isinstance(concurrency, bool) or not isinstance(concurrency, int) or not 1 <= concurrency <= 8:
            raise ProtocolError("max_concurrency must be between 1 and 8")
        version = payload.get("version", "")
        if not isinstance(version, str) or len(version) > 32:
            raise ProtocolError("version must be at most 32 characters")
        self.store.use_challenge(payload.get("challenge"), worker)
        self.store.consume_nonce(worker, nonce, self.config.nonce_ttl_s)
        if self.store.worker(worker) is None and not self.store.is_fleet_enrolled(worker) and \
                self.store.count_workers("pending") >= self.config.max_pending:
            raise ProtocolError("too many contributors are awaiting approval; try again later", status=429,
                                code="pending_full")
        info = self.store.register_worker(worker, label=label, payout_account=payout, gpus=gpus, models=models,
                                          max_concurrency=concurrency, version=version)
        with self._cv:
            self._cv.notify_all()
        return {"status": info["status"], "tier": info["tier"], "reason": info["status_reason"],
                "coordinator": self.identity.public_hex}

    def heartbeat(self, envelope: Any) -> Dict[str, Any]:
        worker, payload = self._open(envelope, "heartbeat")
        if set(payload) - {"gpus", "models"}:
            raise ProtocolError("unexpected heartbeat fields")
        info = self.store.heartbeat(worker, gpus=validate_gpus(payload.get("gpus", [])),
                                    models=validate_models(payload.get("models")))
        with self._cv:
            self._cv.notify_all()
        return {"status": info["status"], "tier": info["tier"], "reason": info["status_reason"]}

    def poll(self, envelope: Any, *, wait_s: Optional[float] = None) -> Optional[Dict[str, Any]]:
        worker, payload = self._open(envelope, "poll")
        if payload:
            raise ProtocolError("poll payload must be empty")
        info = self._require_approved(worker)
        self.store.touch(worker)
        deadline = time.monotonic() + (self.config.poll_wait_s if wait_s is None else wait_s)
        with self._cv:
            self._sweep_canaries()
            info = self.store.worker(worker) or info
            while True:
                queue = self._queues.get(worker)
                while queue:
                    job = queue.popleft()
                    if job.state != "queued" or job.worker != worker:
                        continue
                    if now_ms() >= job.deadline_ms:
                        job.state = "expired"
                        continue
                    job.state = "running"
                    job.started_ms = now_ms()
                    self._running[worker] = self._running.get(worker, 0) + 1
                    self._cv.notify_all()
                    return job.handout()
                canary = self._maybe_canary(info)
                if canary is not None:
                    return canary.handout()
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    return None
                self._cv.wait(timeout=min(remaining, 5))
                info = self.store.worker(worker)
                if info is None or info["status"] != "approved":
                    return None

    def _maybe_canary(self, info: Dict[str, Any]) -> Optional[Job]:
        """Called with the lock held. Public contributors get a canary on first poll, then periodically."""
        if info["tier"] != "public" or not info["models"]:
            return None
        if now_s() - info["last_canary_at"] < self.config.canary_interval_s:
            return None
        worker = info["pubkey"]
        if self._running.get(worker, 0) >= info["max_concurrency"]:
            return None
        model = random.choice(info["models"])["name"]
        code = "X3-" + secrets.token_hex(4).upper()
        request = {
            "model": model,
            "messages": [
                {"role": "system", "content": "You are a careful assistant. Follow the instruction exactly."},
                {"role": "user", "content": f"Reply with only this verification code and nothing else: {code}"},
            ],
            "max_tokens": 64,
            "temperature_milli": 0,
            "top_p_milli": 1000,
        }
        job = Job(job_id=secrets.token_hex(16), kind="canary", app=CANARY_APP, request=request,
                  request_sha256=sha256_hex(canonical(request)),
                  deadline_ms=now_ms() + self.config.job_timeout_s * 1000, canary_code=code, worker=worker,
                  state="running", started_ms=now_ms())
        self._jobs[job.job_id] = job
        self._running[worker] = self._running.get(worker, 0) + 1
        self.store.mark_canary_sent(worker)
        return job

    def _sweep_canaries(self) -> None:
        """Called with the lock held. A canary whose contributor never answered counts as failed."""
        cutoff = now_ms() - self.config.result_grace_s * 1000
        for job in [j for j in self._jobs.values() if j.kind == "canary" and j.state == "running"
                    and j.deadline_ms < cutoff]:
            self._finish(job, "expired")
            self._count_canary(job.worker, False)

    def _finish(self, job: Job, state: str) -> None:
        """Called with the lock held."""
        if job.state == "running" and job.worker:
            self._running[job.worker] = max(0, self._running.get(job.worker, 0) - 1)
        job.state = state
        if job.kind == "canary":
            self._jobs.pop(job.job_id, None)
        self._cv.notify_all()

    def submit_result(self, envelope: Any) -> Dict[str, Any]:
        worker, payload = self._open(envelope, "result")
        expected = {"job_id", "request_sha256", "response", "response_sha256", "model_digest", "result_sig",
                    "started_ms", "finished_ms"}
        if set(payload) != expected:
            raise ProtocolError("result fields must be exactly: " + ", ".join(sorted(expected)))
        info = self._require_approved(worker)
        with self._cv:
            job = self._jobs.get(payload["job_id"]) if isinstance(payload["job_id"], str) else None
            if job is None or job.state != "running":
                raise ProtocolError("no running job with that id", status=409, code="no_such_job")
            if job.worker != worker:
                raise ProtocolError("this job is assigned to a different contributor", status=403,
                                    code="wrong_worker")
            if now_ms() > job.deadline_ms + self.config.result_grace_s * 1000:
                self._finish(job, "expired")
                raise ProtocolError("result arrived after the job deadline", status=409, code="late")
            try:
                self._check_result(job, info, payload)
            except ProtocolError as exc:
                job.error = str(exc)
                self._finish(job, "failed")
                self._count_failure(worker, f"invalid result: {exc}")
                raise
            response = payload["response"]
            record = {
                "job_id": job.job_id, "kind": job.kind, "app": job.app, "worker": worker,
                "worker_tier": info["tier"], "model": job.model, "model_digest": payload["model_digest"],
                "request_sha256": job.request_sha256, "response_sha256": payload["response_sha256"],
                "worker_sig": payload["result_sig"],
                "prompt_tokens": response["usage"]["prompt_tokens"],
                "completion_tokens": response["usage"]["completion_tokens"],
                "started_ms": job.started_ms, "finished_ms": now_ms(),
                "outcome": "ok",
            }
            if job.kind == "canary":
                passed = job.canary_code in response["content"]
                record["outcome"] = "canary_pass" if passed else "canary_fail"
            receipt = self.store.append_receipt(record, lambda h: self.identity.sign(h.encode()))
            job.result = {"response": response, "receipt": receipt, "tier": info["tier"]}
            self._finish(job, "done")
        if job.kind == "canary":
            self._count_canary(worker, record["outcome"] == "canary_pass")
        else:
            self.store.record_job_outcome(worker, True)
        return {"accepted": True, "receipt_hash": receipt["receipt_hash"]}

    def _check_result(self, job: Job, info: Dict[str, Any], payload: Dict[str, Any]) -> None:
        if payload["request_sha256"] != job.request_sha256:
            raise ProtocolError("request_sha256 does not match the job", status=409, code="mismatch")
        response = validate_response(payload["response"])
        if sha256_hex(canonical(response)) != payload["response_sha256"]:
            raise ProtocolError("response_sha256 does not match the response", status=409, code="mismatch")
        if response["model"] != job.model:
            raise ProtocolError("response is for a different model", status=409, code="mismatch")
        digests = {m["name"]: m["digest"] for m in info["models"]}
        if digests.get(job.model) != payload["model_digest"]:
            raise ProtocolError("model_digest does not match the digest this contributor advertised", status=409,
                                code="mismatch")
        message = worker_result_message({"job_id": job.job_id, "request_sha256": job.request_sha256,
                                         "response_sha256": payload["response_sha256"], "model": job.model,
                                         "model_digest": payload["model_digest"]})
        if not verify_signature(info["pubkey"], message.encode(), payload["result_sig"]):
            raise ProtocolError("bad result signature", status=401, code="bad_signature")

    def submit_failure(self, envelope: Any) -> Dict[str, Any]:
        worker, payload = self._open(envelope, "fail")
        if set(payload) != {"job_id", "request_sha256", "reason"}:
            raise ProtocolError("fail fields must be exactly job_id, request_sha256 and reason")
        reason = payload["reason"] if isinstance(payload["reason"], str) else ""
        self._require_approved(worker)
        with self._cv:
            job = self._jobs.get(payload["job_id"]) if isinstance(payload["job_id"], str) else None
            if job is None or job.state != "running" or job.worker != worker:
                raise ProtocolError("no running job with that id for this contributor", status=409,
                                    code="no_such_job")
            if payload["request_sha256"] != job.request_sha256:
                raise ProtocolError("request_sha256 does not match the job", status=409, code="mismatch")
            job.error = reason[:200]
            self._finish(job, "failed")
        if job.kind == "canary":
            self._count_canary(worker, False)
        else:
            self._count_failure(worker, f"reported failure: {reason[:200]}")
        return {"accepted": True}

    def _count_failure(self, worker: str, why: str) -> None:
        info = self.store.record_job_outcome(worker, False)
        if info and info["status"] == "approved" and \
                info["consecutive_failures"] >= self.config.job_failures_to_suspend:
            self.store.set_status(worker, "suspended",
                                  f"auto: {info['consecutive_failures']} failed jobs in a row (last: {why})",
                                  actor="coordinator")

    def _count_canary(self, worker: str, passed: bool) -> None:
        info = self.store.record_canary(worker, passed)
        if not passed and info and info["status"] == "approved" and \
                info["consecutive_canary_fail"] >= self.config.canary_failures_to_suspend:
            self.store.set_status(worker, "suspended",
                                  f"auto: failed {info['consecutive_canary_fail']} canary checks in a row",
                                  actor="coordinator")

    # ---------------------------------------------------------------- app side

    def authenticate(self, header: Optional[str]) -> Dict[str, Any]:
        if not header or not header.startswith("Bearer "):
            raise ProtocolError("missing app key: send Authorization: Bearer x3m_...", status=401,
                                code="invalid_api_key")
        app = self.store.app_for_key(header[len("Bearer "):].strip())
        if app is None:
            raise ProtocolError("unknown or revoked app key", status=401, code="invalid_api_key")
        return app

    def _eligible(self, model: Optional[str], allow_public: bool, exclude: List[str]) -> List[Dict[str, Any]]:
        fresh_after = now_s() - self.config.heartbeat_ttl_s
        out = []
        for info in self.store.workers(status="approved"):
            if info["pubkey"] in exclude:
                continue
            if info["tier"] == "public" and (not allow_public or info["canary_pass"] < 1):
                continue
            if not info["last_heartbeat"] or info["last_heartbeat"] < fresh_after:
                continue
            if model is not None and model not in {m["name"] for m in info["models"]}:
                continue
            out.append(info)
        return out

    def models(self, app: Dict[str, Any]) -> Dict[str, Any]:
        seen: Dict[str, int] = {}
        for info in self._eligible(None, bool(app["allow_public"]), []):
            for m in info["models"]:
                seen[m["name"]] = seen.get(m["name"], 0) + 1
        return {"object": "list", "data": [{"id": name, "object": "model", "owned_by": "x3-gpu-mesh",
                                            "x3_contributors": count} for name, count in sorted(seen.items())]}

    def _reserve(self, app: Dict[str, Any], max_tokens: int) -> None:
        """Rate limit, then reserve ``max_tokens`` against the daily budget until the job ends.

        Counting in-flight reservations means a burst of parallel requests cannot overspend the
        budget; the reservation is released when the job finishes and actual usage is recorded.
        """
        now = time.monotonic()
        used = self.store.tokens_used_today(app["name"])
        with self._cv:
            window = self._rate.setdefault(app["name"], deque())
            while window and now - window[0] > 60:
                window.popleft()
            if len(window) >= app["requests_per_minute"]:
                raise ProtocolError(f"app {app['name']!r} is over {app['requests_per_minute']} requests per minute",
                                    status=429, code="rate_limited")
            reserved = self._reserved.get(app["name"], 0)
            if used + reserved + max_tokens > app["daily_token_budget"]:
                raise ProtocolError(f"app {app['name']!r} has used {used} (plus {reserved} in flight) of its "
                                    f"{app['daily_token_budget']} daily tokens", status=429, code="budget_exhausted")
            window.append(now)
            self._reserved[app["name"]] = reserved + max_tokens

    def _release(self, app: Dict[str, Any], max_tokens: int) -> None:
        with self._cv:
            self._reserved[app["name"]] = max(0, self._reserved.get(app["name"], 0) - max_tokens)

    def complete(self, app: Dict[str, Any], body: Any) -> Tuple[Dict[str, Any], bool]:
        """Run one chat completion. Returns (openai_response, stream_requested)."""
        request, stream = normalize_chat_request(body, max_tokens_cap=app["max_tokens_cap"])
        self._reserve(app, request["max_tokens"])
        job = Job(job_id=secrets.token_hex(16), kind="job", app=app["name"], request=request,
                  request_sha256=sha256_hex(canonical(request)),
                  deadline_ms=now_ms() + self.config.job_timeout_s * 1000)
        allow_public = bool(app["allow_public"])
        with self._cv:
            self._jobs[job.job_id] = job
        try:
            for _attempt in range(self.config.max_attempts):
                candidates = self._eligible(job.model, allow_public, job.tried)
                if not candidates:
                    break
                worker = self._choose(candidates)
                job.tried.append(worker)
                outcome = self._run_on(job, worker)
                if outcome == "done":
                    return self._openai_response(job), stream
                if outcome == "deadline":
                    break
            if not job.tried:
                where = "fleet or approved public" if allow_public else "fleet"
                raise ProtocolError(f"no {where} contributor GPU is serving {job.model} right now", status=503,
                                    code="no_capacity")
            if now_ms() >= job.deadline_ms:
                raise ProtocolError("no contributor finished the job before the deadline", status=504,
                                    code="timeout")
            raise ProtocolError(f"the job failed on {len(job.tried)} contributor(s): {job.error or 'no result'}",
                                status=502, code="worker_failed")
        finally:
            with self._cv:
                self._jobs.pop(job.job_id, None)
            self._release(app, request["max_tokens"])

    def _choose(self, candidates: List[Dict[str, Any]]) -> str:
        with self._cv:
            def load(info: Dict[str, Any]) -> Tuple[int, float, float]:
                busy = self._running.get(info["pubkey"], 0) + len(self._queues.get(info["pubkey"], ()))
                total = info["jobs_ok"] + info["jobs_failed"]
                failure_rate = info["jobs_failed"] / total if total else 0.0
                return (0 if info["tier"] == "fleet" else 1,
                        busy / max(1, info["max_concurrency"]), failure_rate + random.random() * 1e-6)
            return min(candidates, key=load)["pubkey"]

    def _run_on(self, job: Job, worker: str) -> str:
        """Queue the job for one contributor and wait. Returns done | retry | deadline."""
        with self._cv:
            job.worker = worker
            job.state = "queued"
            job.error = None
            self._queues.setdefault(worker, deque()).append(job)
            self._cv.notify_all()
            pickup_by = time.monotonic() + self.config.pickup_timeout_s
            while job.state == "queued":
                remaining = min(pickup_by - time.monotonic(), (job.deadline_ms - now_ms()) / 1000)
                if remaining <= 0:
                    queue = self._queues.get(worker)
                    if queue and job in queue:
                        queue.remove(job)
                    job.state = "unclaimed"
                    return "retry" if now_ms() < job.deadline_ms else "deadline"
                self._cv.wait(timeout=remaining)
            while job.state == "running":
                remaining = (job.deadline_ms - now_ms()) / 1000
                if remaining <= 0:
                    self._finish(job, "expired")
                    break
                self._cv.wait(timeout=remaining)
            state = job.state
        if state == "done":
            return "done"
        if state == "expired":
            self._count_failure(worker, "did not finish before the deadline")
            return "deadline"
        return "retry" if now_ms() < job.deadline_ms else "deadline"

    def _openai_response(self, job: Job) -> Dict[str, Any]:
        assert job.result is not None
        response, receipt = job.result["response"], job.result["receipt"]
        return {
            "id": f"x3mesh-{job.job_id}",
            "object": "chat.completion",
            "created": receipt["finished_ms"] // 1000,
            "model": job.model,
            "choices": [{"index": 0, "message": {"role": "assistant", "content": response["content"]},
                         "finish_reason": response["finish_reason"]}],
            "usage": {"prompt_tokens": response["usage"]["prompt_tokens"],
                      "completion_tokens": response["usage"]["completion_tokens"],
                      "total_tokens": response["usage"]["prompt_tokens"] + response["usage"]["completion_tokens"]},
            "x3_mesh": {
                "protocol": PROTOCOL,
                "untrusted_output": True,
                "job_id": job.job_id,
                "app": job.app,
                "worker": receipt["worker"],
                "worker_tier": receipt["worker_tier"],
                "model": receipt["model"],
                "model_digest": receipt["model_digest"],
                "request_sha256": receipt["request_sha256"],
                "response_sha256": receipt["response_sha256"],
                "worker_sig": receipt["worker_sig"],
                "receipt_seq": receipt["seq"],
                "receipt_hash": receipt["receipt_hash"],
                "prev_hash": receipt["prev_hash"],
                "coordinator": self.identity.public_hex,
                "coordinator_sig": receipt["coordinator_sig"],
            },
        }

    def health(self) -> Dict[str, Any]:
        ready = len(self._eligible(None, False, [])) + len([w for w in self._eligible(None, True, [])
                                                              if w["tier"] == "public"])
        return {"ok": True, "protocol": PROTOCOL, "version": __version__, "contributors_ready": ready,
                "coordinator": self.identity.public_hex}


# --------------------------------------------------------------------------------------------
# HTTP
# --------------------------------------------------------------------------------------------

def _sse_chunks(response: Dict[str, Any], include_usage: bool) -> List[Dict[str, Any]]:
    """The whole answer as OpenAI-style stream chunks (one content chunk, then the finish chunk)."""
    base = {"id": response["id"], "object": "chat.completion.chunk", "created": response["created"],
            "model": response["model"]}
    choice = response["choices"][0]
    chunks = [
        {**base, "choices": [{"index": 0, "delta": {"role": "assistant", "content": choice["message"]["content"]},
                              "finish_reason": None}]},
        {**base, "choices": [{"index": 0, "delta": {}, "finish_reason": choice["finish_reason"]}],
         "x3_mesh": response["x3_mesh"]},
    ]
    if include_usage:
        chunks.append({**base, "choices": [], "usage": response["usage"]})
    return chunks


class MeshServer(ThreadingHTTPServer):
    daemon_threads = True
    allow_reuse_address = True

    def __init__(self, address: Tuple[str, int], coordinator: Coordinator):
        super().__init__(address, MeshHandler)
        self.coordinator = coordinator


class MeshHandler(BaseHTTPRequestHandler):
    server: MeshServer
    server_version = "x3-gpu-mesh"
    sys_version = ""
    protocol_version = "HTTP/1.1"
    timeout = 30  # socket read/write timeout; long-poll waits happen off the socket

    def log_message(self, fmt: str, *args: Any) -> None:  # never log bodies (prompts) or headers (keys)
        log.info("%s %s", self.address_string(), fmt % args)

    def _send_json(self, status: int, payload: Dict[str, Any]) -> None:
        body = json.dumps(payload).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", "no-store")
        self.send_header("X-Content-Type-Options", "nosniff")
        self.end_headers()
        self.wfile.write(body)

    def _send_error(self, exc: ProtocolError) -> None:
        self.close_connection = True  # the request body may be unread; never reuse this connection
        default = "invalid_request_error" if exc.status < 500 else "server_error"
        kind = {401: "authentication_error", 403: "permission_error", 404: "not_found_error",
                429: "rate_limit_error"}.get(exc.status, default)
        self._send_json(exc.status, {"error": {"message": str(exc), "type": kind, "code": exc.code}})

    def _read_json(self) -> Any:
        length = self.headers.get("Content-Length")
        if length is None:
            raise ProtocolError("Content-Length required", status=411, code="length_required")
        try:
            size = int(length)
        except ValueError:
            raise ProtocolError("bad Content-Length") from None
        if size < 0 or size > MAX_BODY_BYTES:
            # Drain nothing: closing the connection is the safe answer to an oversized body.
            self.close_connection = True
            raise ProtocolError(f"body larger than {MAX_BODY_BYTES} bytes", status=413, code="too_large")
        raw = self.rfile.read(size)
        try:
            return json.loads(raw.decode("utf-8"))
        except (UnicodeDecodeError, json.JSONDecodeError):
            raise ProtocolError("body is not valid JSON") from None

    def do_GET(self) -> None:  # noqa: N802
        path = urlparse(self.path).path
        coord = self.server.coordinator
        try:
            if path == "/healthz":
                self._send_json(200, coord.health())
            elif path == "/mesh/challenge":
                self._send_json(200, coord.store.new_challenge(max_open=coord.config.max_open_challenges))
            elif path in ("/v1/models", "/models"):
                app = coord.authenticate(self.headers.get("Authorization"))
                self._send_json(200, coord.models(app))
            else:
                raise ProtocolError("not found", status=404, code="not_found")
        except ProtocolError as exc:
            self._send_error(exc)
        except Exception:  # pragma: no cover - defensive
            log.exception("GET %s failed", path)
            self._send_error(ProtocolError("internal error", status=500, code="internal"))

    def do_POST(self) -> None:  # noqa: N802
        path = urlparse(self.path).path
        coord = self.server.coordinator
        try:
            if path in ("/v1/chat/completions", "/chat/completions"):
                app = coord.authenticate(self.headers.get("Authorization"))
                body = self._read_json()
                response, stream = coord.complete(app, body)
                if stream:
                    include_usage = bool(isinstance(body.get("stream_options"), dict)
                                         and body["stream_options"].get("include_usage"))
                    self._send_stream(response, include_usage)
                else:
                    self._send_json(200, response)
                return
            body = self._read_json()
            if path == "/mesh/register":
                self._send_json(200, coord.register(body))
            elif path == "/mesh/heartbeat":
                self._send_json(200, coord.heartbeat(body))
            elif path == "/mesh/poll":
                job = coord.poll(body)
                if job is None:
                    self._send_json(200, {"job": None})
                else:
                    self._send_json(200, {"job": job})
            elif path == "/mesh/result":
                self._send_json(200, coord.submit_result(body))
            elif path == "/mesh/fail":
                self._send_json(200, coord.submit_failure(body))
            else:
                raise ProtocolError("not found", status=404, code="not_found")
        except ProtocolError as exc:
            self._send_error(exc)
        except (BrokenPipeError, ConnectionResetError):
            pass
        except Exception:  # pragma: no cover - defensive
            log.exception("POST %s failed", path)
            self._send_error(ProtocolError("internal error", status=500, code="internal"))

    def _send_stream(self, response: Dict[str, Any], include_usage: bool) -> None:
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.send_header("Cache-Control", "no-store")
        self.send_header("Connection", "close")
        self.end_headers()
        self.close_connection = True
        for chunk in _sse_chunks(response, include_usage):
            self.wfile.write(b"data: " + json.dumps(chunk).encode("utf-8") + b"\n\n")
        self.wfile.write(b"data: [DONE]\n\n")
        self.wfile.flush()


def serve(coordinator: Coordinator, host: str = "127.0.0.1", port: int = 8790) -> MeshServer:
    server = MeshServer((host, port), coordinator)
    log.info("x3 gpu mesh coordinator %s listening on %s:%d", coordinator.identity.public_hex, host,
             server.server_address[1])
    return server
