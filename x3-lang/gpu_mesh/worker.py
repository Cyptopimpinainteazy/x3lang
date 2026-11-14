"""The contributor daemon: lends this machine's GPU to X3 agents through a local Ollama.

It only makes outbound HTTPS requests to the coordinator (long-polling for work), so a
contributor behind NAT or a home router needs no open ports. It runs nothing but inference:
the job is a list of chat messages, which go to the local Ollama's OpenAI-compatible endpoint
and nowhere else. No shell, no tools, no file access.

Every message it sends is signed with this machine's Ed25519 key. The key file is created
with mode 0600 on first run and is the contributor's identity: keep it, and don't copy it to
another machine (two machines with one key look like one contributor).
"""

from __future__ import annotations

import ipaddress
import json
import logging
import socket
import threading
import urllib.error
import urllib.request
from typing import Any, Callable, Dict, List, Optional, Tuple
from urllib.parse import urlparse

from . import __version__, gpu_probe
from .protocol import (Identity, ProtocolError, canonical, now_ms, seal, sha256_hex, validate_job_request,
                       validate_label, validate_models, validate_payout_account, validate_response)
from .store import worker_result_message

log = logging.getLogger("x3.gpu_mesh.worker")


class TransportError(RuntimeError):
    pass


def _is_loopback(host: str) -> bool:
    if host in ("localhost",):
        return True
    try:
        return ipaddress.ip_address(host).is_loopback
    except ValueError:
        return False


def check_url(url: str, *, allow_http: bool, what: str) -> str:
    parsed = urlparse(url)
    if parsed.scheme not in ("http", "https") or not parsed.hostname:
        raise ValueError(f"{what} must be an http(s) URL, got {url!r}")
    if parsed.scheme == "http" and not (allow_http or _is_loopback(parsed.hostname)):
        raise ValueError(f"{what} {url} is plain http on a non-loopback host; use https, or pass "
                         "--allow-http on a private network you control")
    return url.rstrip("/")


def http_json(method: str, url: str, body: Optional[Any] = None, *, timeout: float = 30.0,
              headers: Optional[Dict[str, str]] = None) -> Tuple[int, Any]:
    data = json.dumps(body).encode("utf-8") if body is not None else None
    base_headers = {"Content-Type": "application/json", "User-Agent": f"x3-gpu-mesh/{__version__}"}
    req = urllib.request.Request(url, data=data, method=method, headers={**base_headers, **(headers or {})})
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:  # noqa: S310 - URL scheme checked by check_url
            raw = resp.read(4 * 1024 * 1024)
            status = resp.status
    except urllib.error.HTTPError as exc:
        raw = exc.read(64 * 1024)
        status = exc.code
    except (urllib.error.URLError, socket.timeout, ConnectionError, TimeoutError, OSError) as exc:
        raise TransportError(f"{method} {url}: {exc}") from exc
    try:
        return status, json.loads(raw.decode("utf-8")) if raw else None
    except (UnicodeDecodeError, json.JSONDecodeError):
        return status, None


class OllamaClient:
    """Talks to the contributor's local Ollama (or any server with Ollama's /api/tags and an
    OpenAI-compatible /v1/chat/completions)."""

    def __init__(self, base_url: str, *, allow_http: bool = False, http: Callable = http_json):
        self.base_url = check_url(base_url, allow_http=allow_http, what="Ollama URL")
        self.http = http

    def models(self, allow: Optional[List[str]] = None) -> List[Dict[str, str]]:
        status, data = self.http("GET", f"{self.base_url}/api/tags", timeout=10)
        if status != 200 or not isinstance(data, dict):
            raise TransportError(f"Ollama /api/tags answered {status}")
        found = []
        for entry in data.get("models") or []:
            name = entry.get("name") or entry.get("model")
            digest = entry.get("digest")
            if not isinstance(name, str) or not isinstance(digest, str):
                continue
            if allow and name not in allow:
                continue
            found.append({"name": name, "digest": digest.lower()})
        return validate_models(found[:64]) if found else []

    def chat(self, request: Dict[str, Any], timeout: float) -> Dict[str, Any]:
        body: Dict[str, Any] = {
            "model": request["model"],
            "messages": request["messages"],
            "max_tokens": request["max_tokens"],
            "temperature": request["temperature_milli"] / 1000,
            "top_p": request["top_p_milli"] / 1000,
            "stream": False,
        }
        if "seed" in request:
            body["seed"] = request["seed"]
        if "stop" in request:
            body["stop"] = request["stop"]
        status, data = self.http("POST", f"{self.base_url}/v1/chat/completions", body, timeout=timeout)
        if status != 200 or not isinstance(data, dict):
            message = ""
            if isinstance(data, dict):
                err = data.get("error")
                message = err.get("message") if isinstance(err, dict) else str(err or "")
            raise TransportError(f"Ollama answered {status} {message[:160]}")
        choice = (data.get("choices") or [{}])[0] or {}
        content = (choice.get("message") or {}).get("content")
        if not isinstance(content, str):
            raise TransportError("Ollama returned no text content")
        usage = data.get("usage") if isinstance(data.get("usage"), dict) else {}

        def count(key: str) -> int:
            value = usage.get(key, 0)
            return value if isinstance(value, int) and not isinstance(value, bool) and value >= 0 else 0

        return validate_response({
            "content": content,
            "finish_reason": "length" if choice.get("finish_reason") == "length" else "stop",
            "model": request["model"],
            "usage": {"prompt_tokens": count("prompt_tokens"), "completion_tokens": count("completion_tokens")},
        })


class Worker:
    def __init__(self, identity: Identity, coordinator_url: str, ollama: OllamaClient, *, label: str,
                 payout_account: Optional[str] = None, models: Optional[List[str]] = None, max_concurrency: int = 1,
                 allow_http: bool = False, heartbeat_s: float = 30.0, http: Callable = http_json,
                 gpus: Optional[Callable[[], List[Dict[str, Any]]]] = None, idle_wait_s: float = 15.0):
        self.identity = identity
        self.coordinator = check_url(coordinator_url, allow_http=allow_http, what="coordinator URL")
        self.ollama = ollama
        self.label = validate_label(label)
        self.payout_account = validate_payout_account(payout_account)
        self.allow_models = models
        if not 1 <= max_concurrency <= 8:
            raise ValueError("max_concurrency must be between 1 and 8")
        self.max_concurrency = max_concurrency
        self.heartbeat_s = heartbeat_s
        self.idle_wait_s = idle_wait_s
        self.http = http
        self.probe_gpus = gpus or gpu_probe.probe
        self.stop_event = threading.Event()
        self.status: Optional[str] = None
        self.reason: str = ""
        self._models: List[Dict[str, str]] = []
        self._threads: List[threading.Thread] = []
        self.jobs_done = 0

    # ---------------------------------------------------------------- messages

    def _post(self, path: str, kind: str, payload: Dict[str, Any], timeout: float = 30.0) -> Tuple[int, Any]:
        return self.http("POST", f"{self.coordinator}{path}", seal(self.identity, kind, payload), timeout=timeout)

    def _refresh_models(self) -> List[Dict[str, str]]:
        models = self.ollama.models(self.allow_models)
        if not models:
            raise TransportError("Ollama has none of the allowed models pulled; run `ollama pull <model>`")
        self._models = models
        return models

    def register(self) -> Dict[str, Any]:
        models = self._refresh_models()
        status, challenge = self.http("GET", f"{self.coordinator}/mesh/challenge", timeout=15)
        if status != 200 or not isinstance(challenge, dict):
            raise TransportError(f"challenge request answered {status}")
        status, data = self._post("/mesh/register", "register", {
            "challenge": challenge["challenge"], "label": self.label, "payout_account": self.payout_account,
            "gpus": self.probe_gpus(), "models": models, "max_concurrency": self.max_concurrency,
            "version": __version__,
        })
        if status != 200:
            raise TransportError(f"register answered {status}: {_err(data)}")
        self.status, self.reason = data["status"], data.get("reason", "")
        log.info("registered as %s (%s): %s", self.identity.public_hex, self.status, self.reason)
        return data

    def heartbeat_once(self) -> None:
        models = self._refresh_models()
        status, data = self._post("/mesh/heartbeat", "heartbeat", {"gpus": self.probe_gpus(), "models": models})
        if status == 200:
            self.status, self.reason = data["status"], data.get("reason", "")
        else:
            raise TransportError(f"heartbeat answered {status}: {_err(data)}")

    def poll_once(self, wait_timeout: float = 40.0) -> bool:
        """One long-poll. Returns True if a job was run."""
        status, data = self._post("/mesh/poll", "poll", {}, timeout=wait_timeout)
        if status == 403:
            code = (data or {}).get("error", {}).get("code") if isinstance(data, dict) else None
            self.status = code or "forbidden"
            self.reason = _err(data)
            return False
        if status != 200 or not isinstance(data, dict):
            raise TransportError(f"poll answered {status}: {_err(data)}")
        job = data.get("job")
        if not job:
            return False
        self.status = "approved"
        self.run_job(job)
        return True

    def run_job(self, job: Dict[str, Any]) -> None:
        job_id = job.get("job_id")
        request_sha = job.get("request_sha256")
        try:
            if not isinstance(job_id, str) or not isinstance(request_sha, str):
                raise ProtocolError("malformed job")
            request = validate_job_request(job.get("request"))
            if sha256_hex(canonical(request)) != request_sha:
                raise ProtocolError("request_sha256 does not match the request")
            digests = {m["name"]: m["digest"] for m in self._models}
            if request["model"] not in digests:
                raise ProtocolError(f"model {request['model']} is not served here")
            deadline = job.get("deadline_ms")
            remaining = (deadline - now_ms()) / 1000 if isinstance(deadline, int) else 60
            if remaining <= 1:
                raise ProtocolError("job deadline already passed")
            started = now_ms()
            response = self.ollama.chat(request, timeout=max(1.0, remaining - 1))
            finished = now_ms()
        except (ProtocolError, TransportError) as exc:
            log.warning("job %s failed: %s", job_id, exc)
            if isinstance(job_id, str) and isinstance(request_sha, str):
                try:
                    self._post("/mesh/fail", "fail", {"job_id": job_id, "request_sha256": request_sha,
                                                       "reason": str(exc)[:200]})
                except TransportError:
                    pass
            return
        response_sha = sha256_hex(canonical(response))
        digest = digests[request["model"]]
        result_sig = self.identity.sign(worker_result_message({
            "job_id": job_id, "request_sha256": request_sha, "response_sha256": response_sha,
            "model": request["model"], "model_digest": digest}).encode())
        status, data = self._post("/mesh/result", "result", {
            "job_id": job_id, "request_sha256": request_sha, "response": response, "response_sha256": response_sha,
            "model_digest": digest, "result_sig": result_sig, "started_ms": started, "finished_ms": finished,
        })
        if status == 200:
            self.jobs_done += 1
            log.info("job %s done (%d completion tokens)", job_id, response["usage"]["completion_tokens"])
        else:
            log.warning("result for job %s refused (%s): %s", job_id, status, _err(data))

    # ---------------------------------------------------------------- loops

    def _heartbeat_loop(self) -> None:
        while not self.stop_event.wait(self.heartbeat_s):
            try:
                self.heartbeat_once()
            except (TransportError, ProtocolError) as exc:
                log.warning("heartbeat failed: %s", exc)

    def _poll_loop(self) -> None:
        backoff = 1.0
        while not self.stop_event.is_set():
            try:
                self.poll_once()
                backoff = 1.0
                if self.status not in (None, "approved"):
                    log.info("not serving yet: %s (%s)", self.status, self.reason)
                    self.stop_event.wait(self.idle_wait_s)
            except (TransportError, ProtocolError) as exc:
                log.warning("poll failed: %s; retrying in %.0fs", exc, backoff)
                self.stop_event.wait(backoff)
                backoff = min(30.0, backoff * 2)

    def start(self) -> None:
        backoff = 2.0
        while not self.stop_event.is_set():
            try:
                self.register()
                break
            except (TransportError, ProtocolError, ValueError) as exc:
                log.warning("register failed: %s; retrying in %.0fs", exc, backoff)
                self.stop_event.wait(backoff)
                backoff = min(60.0, backoff * 2)
        if self.stop_event.is_set():
            return
        self._threads = [threading.Thread(target=self._heartbeat_loop, name="x3-mesh-heartbeat", daemon=True)]
        self._threads += [threading.Thread(target=self._poll_loop, name=f"x3-mesh-poll-{i}", daemon=True)
                          for i in range(self.max_concurrency)]
        for thread in self._threads:
            thread.start()

    def stop(self) -> None:
        self.stop_event.set()

    def join(self, timeout: Optional[float] = None) -> None:
        for thread in self._threads:
            thread.join(timeout)

    def run_forever(self) -> None:
        self.start()
        try:
            while not self.stop_event.wait(1):
                pass
        except KeyboardInterrupt:
            self.stop()
        self.join(5)


def _err(data: Any) -> str:
    if isinstance(data, dict) and isinstance(data.get("error"), dict):
        return str(data["error"].get("message", ""))[:300]
    return ""
