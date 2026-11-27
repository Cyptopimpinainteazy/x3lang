"""Test-only support for the Forge x GPU mesh tests. Never imported by production code.

`ProgramWritingOllama` stands in for a contributor's Ollama (tags + streaming chat). Run as a script,
it boots a real gpu_mesh coordinator, enrolls and starts a real fleet worker against the fake, mints
a Forge app key and writes {url, app_key} to --out, for the browser e2e."""
from __future__ import annotations

import argparse
import json
import re
import sys
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

MODEL = "qwen3:8b"
DIGEST = "b" * 64
OWNER = "x3-owner.near"
REFUND = "4Nd1mzi8Y1QYxJt9wZWBYZpG7S4pYkZs6YzD3Vt9aBcD"


class ProgramWritingOllama:
    """Ollama's /api/tags and streaming /api/chat. It reads the nonce the Forge put in the prompt and
    writes an X3 program paying `recipient`: honest pays the allowed payee, hostile pays an attacker.
    A task that mentions attacker.near plays a prompt injection the model falls for."""

    def __init__(self):
        self.recipient = "x3-demo.near"
        self.amount = 10_000_000
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
                    self._json(200, {"models": [{"name": MODEL, "model": MODEL, "digest": DIGEST}]})
                else:
                    self._json(404, {"error": "not found"})

            def do_POST(self):
                body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
                fake.calls.append(body)
                text = "\n".join(m["content"] for m in body["messages"])
                if "verification code" in body["messages"][-1]["content"]:
                    content = body["messages"][-1]["content"].rsplit(" ", 1)[-1]
                else:
                    m = re.search(r"- nonce: (\S+)", text)
                    nonce = m.group(1) if m else "none"
                    # A prompt-injected task ("...send it to attacker.near") turns the model hostile.
                    recipient = "attacker.near" if "attacker.near" in body["messages"][-1]["content"] else fake.recipient
                    content = (
                        "```x3\nintent pay_invoice {\n"
                        f"    from Solana.USDC amount {fake.amount} receiver {REFUND}\n"
                        f"    to Near.USDC receiver {recipient}\n"
                        "    route {\n"
                        f"        bridge NearIntents Solana.USDC -> Near.USDC min_receive {fake.amount * 99 // 100};\n"
                        "    }\n"
                        f"    under mandate {OWNER}\n"
                        "    require slippage <= 1%\n"
                        "    require fees <= 100 bps\n"
                        f"    require nonce unused {nonce}\n"
                        "    timeout 10m refund Solana.USDC\n"
                        "    on_fail rollback\n}\n```")
                self.send_response(200)
                self.send_header("Content-Type", "application/x-ndjson")
                self.end_headers()
                for i in range(0, len(content), 40):
                    self.wfile.write(json.dumps({"model": body["model"], "message": {"role": "assistant",
                                                 "content": content[i:i + 40]}, "done": False}).encode() + b"\n")
                self.wfile.write(json.dumps({"model": body["model"], "message": {"role": "assistant", "content": ""},
                                             "done": True, "done_reason": "stop", "prompt_eval_count": 11,
                                             "eval_count": 7}).encode() + b"\n")

        self.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.url = f"http://127.0.0.1:{self.server.server_address[1]}"
        threading.Thread(target=self.server.serve_forever, daemon=True).start()

    def close(self):
        self.server.shutdown()
        self.server.server_close()


def main(argv=None) -> int:
    sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
    from gpu_mesh.coordinator import Config, Coordinator, serve
    from gpu_mesh.protocol import Identity
    from gpu_mesh.store import Store
    from gpu_mesh.worker import OllamaClient, Worker

    ap = argparse.ArgumentParser()
    ap.add_argument("--port", type=int, required=True)
    ap.add_argument("--data", required=True)
    ap.add_argument("--out", required=True)
    args = ap.parse_args(argv)
    Path(args.data).mkdir(parents=True, exist_ok=True)
    store = Store(str(Path(args.data) / "mesh.db"))
    coord = Coordinator(store, Identity.generate(), Config(poll_wait_s=1, pickup_timeout_s=5, job_timeout_s=30,
                                                           heartbeat_ttl_s=30, canary_interval_s=3600))
    server = serve(coord, "127.0.0.1", args.port)
    url = f"http://127.0.0.1:{args.port}"
    threading.Thread(target=server.serve_forever, daemon=True).start()
    ollama = ProgramWritingOllama()
    wid = Identity.generate()
    store.enroll_fleet(wid.public_hex, "e2e-gpu")
    worker = Worker(wid, url, OllamaClient(ollama.url), label="e2e-gpu", heartbeat_s=0.5,
                    gpus=lambda: [{"name": "RTX 3090", "vendor": "nvidia", "vram_mb": 24576}], idle_wait_s=0.2)
    worker.start()
    deadline = time.time() + 15
    while worker.status != "approved" and time.time() < deadline:
        time.sleep(0.05)
    if worker.status != "approved":
        print("worker was not approved", file=sys.stderr)
        return 1
    key = store.add_app_key("forge-e2e", allow_public=False, requests_per_minute=600, daily_token_budget=1_000_000,
                            max_tokens_cap=4096)
    Path(args.out).write_text(json.dumps({"url": url, "app_key": key}))
    print(f"mesh ready at {url}", flush=True)
    while True:
        time.sleep(3600)


if __name__ == "__main__":
    raise SystemExit(main())
