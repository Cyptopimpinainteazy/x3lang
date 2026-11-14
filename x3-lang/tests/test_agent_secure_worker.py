"""Linux-only integration tests: signed task crosses a real subprocess Unix socket."""
import base64
import json
import multiprocessing as mp
import os
from pathlib import Path
import socket
import stat
import struct
import tempfile
import time
import unittest
from unittest.mock import patch

from cryptography.hazmat.primitives import serialization
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey

from agent_constitution import AgentManifest
from agent_runtime import Action, Adapter, Runtime, StampGatePolicy
from agent_tool_gateway import LocalToolGateway
from agent_secure_worker import serve_unix_worker, request_unix_worker, MAX_WIRE_BYTES


def pub(key):
    return key.public_key().public_bytes(serialization.Encoding.Raw, serialization.PublicFormat.Raw)


def start_worker(path, db, operator_key, issuer_key, policy, allowed_uids, ready, stop, request_deadline=2.0, delay=0, client_group=None):
    # Execution callback is registered only inside the worker process.
    adapter = Adapter("compile", "local-plan", 1, False,
                      lambda source: (time.sleep(delay) or (str(os.getpid()) + ":" + source.decode("ascii")).encode()),
                      policy_id="v1")
    runtime = Runtime(db, pub(operator_key), {"compile-intent": adapter},
                      stamp_policies={"compile-intent": policy}, stamp_enforcement="all",
                      clock=lambda: 100)
    gateway = LocalToolGateway(runtime, {"compile-intent": issuer_key})
    serve_unix_worker(gateway, path, allowed_uids=allowed_uids, ready_event=ready,
                      stop_event=stop, request_deadline=request_deadline, client_group=client_group)


@unittest.skipUnless(os.name == "posix" and hasattr(socket, "SO_PEERCRED"), "Linux SO_PEERCRED")
class SecureWorkerTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.root.chmod(0o700)
        self.path = self.root / "worker.sock"
        self.private = self.root / "worker-private"
        self.private.mkdir(mode=0o700)
        self.db = self.private / "agent.sqlite"
        self.operator = Ed25519PrivateKey.generate()
        self.agent = Ed25519PrivateKey.generate()
        self.issuer = Ed25519PrivateKey.generate()
        self.policy = StampGatePolicy(pub(self.issuer), "a" * 64,
                        ("signed_agent_identity", "capability_scope", "source_digest"))
        adapter = Adapter("compile", "local-plan", 1, False,
                          lambda src: b"never-called-in-parent", policy_id="v1")
        self.runtime = Runtime(self.db, pub(self.operator), {"compile-intent": adapter},
                               stamp_policies={"compile-intent": self.policy},
                               stamp_enforcement="all", clock=lambda: 100)
        self.db.chmod(0o600)
        manifest = AgentManifest("agent-one", None, frozenset({"compile"}),
                                  frozenset({"local-plan"}), 10, 500, (), 0)
        birth = self.runtime.genesis_payload(manifest, pub(self.agent), "birth-one")
        self.runtime.genesis(manifest, pub(self.agent), "birth-one", self.operator.sign(birth))
        self.context = mp.get_context("fork")
        self.ready = self.context.Event()
        self.stop = self.context.Event()
        self.process = None

    def tearDown(self):
        if self.process is not None:
            self.stop.set()
            self.process.join(4)
            if self.process.is_alive():
                self.process.terminate()
                self.process.join(4)
            self.assertFalse(self.process.is_alive(), "worker failed to exit")

    def start_server(self, allowed_uids=None, *, request_deadline=2.0, delay=0, client_group=None):
        self.process = self.context.Process(target=start_worker,
                    args=(str(self.path), self.db, self.operator, self.issuer,
                          self.policy, allowed_uids, self.ready, self.stop, request_deadline, delay, client_group))
        self.process.start()
        self.assertTrue(self.ready.wait(5), "worker failed to bind Unix socket")
        self.assertTrue(self.process.is_alive(), "worker exited unexpectedly")

    def signed(self, nonce="one", source=b"safe"):
        act = Action("agent-one", "compile-intent", source, nonce, 150)
        return json.dumps({
            "agent_id": act.agent_id, "adapter": act.adapter, "nonce": act.nonce,
            "expires_at": act.expires_at, "artifact_b64": base64.b64encode(source).decode(),
            "agent_signature_hex": self.agent.sign(self.runtime.action_payload(act)).hex(),
        }).encode()

    def test_separate_worker_process_authorized_request(self):
        self.start_server(allowed_uids={os.getuid()})
        result = request_unix_worker(self.path, self.signed())
        self.assertEqual(result, str(self.process.pid).encode() + b":safe")
        self.assertNotEqual(self.process.pid, os.getpid())
        self.assertEqual(stat.S_IMODE(self.path.stat().st_mode), 0o600)

    def test_unlisted_peer_uid_fails_before_dispatch(self):
        self.start_server(allowed_uids={os.getuid() + 1})
        with self.assertRaises(ValueError):
            request_unix_worker(self.path, self.signed())
        self.assertEqual(self.runtime.snapshot("agent-one")["remaining"], 10)

    def test_forged_signature_denied_no_execution(self):
        self.start_server(allowed_uids={os.getuid()})
        message = json.loads(self.signed())
        message["agent_signature_hex"] = "00" * 64
        with self.assertRaises(ValueError):
            request_unix_worker(self.path, json.dumps(message).encode())
        self.assertEqual(self.runtime.snapshot("agent-one")["remaining"], 10)

    def test_replay_is_rejected_across_process_boundary(self):
        self.start_server(allowed_uids={os.getuid()})
        request = self.signed()
        request_unix_worker(self.path, request)
        with self.assertRaises(ValueError):
            request_unix_worker(self.path, request)
        self.assertEqual(self.runtime.snapshot("agent-one")["remaining"], 9)

    def test_oversized_frame_is_rejected(self):
        self.start_server(allowed_uids={os.getuid()})
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as sock:
            sock.settimeout(2)
            sock.connect(str(self.path))
            sock.sendall(struct.pack("!I", MAX_WIRE_BYTES + 1))
            data = sock.recv(10)
        self.assertTrue(data, "worker should return bounded denial")

    def test_untrusted_socket_directory_permissions_rejected(self):
        self.root.chmod(0o777)
        with self.assertRaises(ValueError):
            serve_unix_worker(None, self.path, allowed_uids={os.getuid()},
                              ready_event=self.ready, stop_event=self.stop)

    def test_shutdown_removes_only_owned_socket(self):
        self.start_server(allowed_uids={os.getuid()})
        self.assertTrue(self.path.exists())
        self.stop.set()
        self.process.join(4)
        self.assertFalse(self.process.is_alive())
        self.assertFalse(self.path.exists())

    def test_denial_never_discloses_private_signing_material(self):
        self.start_server(allowed_uids={os.getuid()})
        forged = json.loads(self.signed())
        forged["agent_signature_hex"] = "00" * 64
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as sock:
            sock.settimeout(2)
            sock.connect(str(self.path))
            raw = json.dumps(forged).encode()
            sock.sendall(struct.pack("!I", len(raw)) + raw)
            response = sock.recv(512)
        self.assertIn(b"request denied", response)
        self.assertNotIn(b"private", response)
        self.assertNotIn(b"agent-one", response)

    def test_client_refuses_oversized_request_without_connecting(self):
        with self.assertRaisesRegex(ValueError, "invalid request size"):
            request_unix_worker(self.path, b"x" * (MAX_WIRE_BYTES + 1))

    def test_incomplete_request_body_does_not_execute(self):
        self.start_server(allowed_uids={os.getuid()})
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as sock:
            sock.settimeout(3)
            sock.connect(str(self.path))
            sock.sendall(struct.pack("!I", 30) + b"partial")
            sock.shutdown(socket.SHUT_WR)
            response = sock.recv(512)
        self.assertIn(b"request denied", response)
        self.assertEqual(self.runtime.snapshot("agent-one")["remaining"], 10)

    def test_drip_feed_reaches_total_deadline_and_other_request_progresses(self):
        self.start_server(allowed_uids={os.getuid()}, request_deadline=0.35)
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as slow:
            slow.settimeout(2)
            slow.connect(str(self.path))
            # A valid frame size but no complete request. Send bytes slowly
            # enough to keep the per-recv idle timeout reset.
            slow.sendall(struct.pack("!I", 200) + b"{")
            time.sleep(0.17)
            slow.sendall(b" ")
            time.sleep(0.28)
            # Worker must release the sequential connection by absolute deadline.
            self.assertIn(b"request denied", slow.recv(100))
        self.assertEqual(request_unix_worker(self.path, self.signed()), 
                         str(self.process.pid).encode() + b":safe")

    def test_slow_approved_execution_can_use_separate_result_wait(self):
        self.start_server(allowed_uids={os.getuid()}, delay=2.2)
        result = request_unix_worker(self.path, self.signed(), execution_timeout=4.0)
        self.assertEqual(result, str(self.process.pid).encode() + b":safe")

    def test_invalid_execution_timeout_is_rejected_before_connecting(self):
        for invalid in (-1, 0, float("nan"), "forever"):
            with self.subTest(invalid=invalid), self.assertRaises(ValueError):
                request_unix_worker(self.path, self.signed(), execution_timeout=invalid)

    def test_group_mode_disallows_worker_uid_on_client_allowlist(self):
        self.root.chmod(0o2710)
        with self.assertRaisesRegex(ValueError, "distinct client UID"):
            gateway = LocalToolGateway(self.runtime, {"compile-intent": self.issuer})
            serve_unix_worker(gateway, self.path, allowed_uids={os.geteuid()},
                              stop_event=self.stop, client_group=os.getegid())

    def test_group_mode_rejects_missing_setgid_directory(self):
        self.root.chmod(0o710)
        with self.assertRaises(ValueError):
            serve_unix_worker(None, self.path, allowed_uids={os.geteuid()+1},
                              stop_event=self.stop, client_group=os.getegid())

    def test_group_mode_refuses_database_directory_accessible_by_clients(self):
        from agent_tool_gateway import LocalToolGateway
        self.root.chmod(0o2710)
        self.private.chmod(0o710)
        gateway = LocalToolGateway(self.runtime, {"compile-intent": self.issuer})
        self.stop.set()
        with self.assertRaisesRegex(ValueError, "worker database directory must be private"):
            serve_unix_worker(gateway, self.path, allowed_uids={os.geteuid()+1},
                              stop_event=self.stop, client_group=os.getegid())
        self.assertFalse(self.path.exists())

    def test_group_mode_refuses_database_world_readable_file(self):
        from agent_tool_gateway import LocalToolGateway
        self.root.chmod(0o2710)
        self.db.chmod(0o644)
        gateway = LocalToolGateway(self.runtime, {"compile-intent": self.issuer})
        self.stop.set()
        with self.assertRaisesRegex(ValueError, "worker database file must be private"):
            serve_unix_worker(gateway, self.path, allowed_uids={os.geteuid()+1},
                              stop_event=self.stop, client_group=os.getegid())
        self.assertFalse(self.path.exists())

    def test_group_mode_binds_separate_uid_socket_permissions(self):
        self.root.chmod(0o2710)
        # This checks kernel peer enforcement as the current UID is refused.
        # It cannot establish cross-UID access without a second OS user.
        self.start_server(allowed_uids={os.geteuid()+1}, client_group=os.getegid())
        self.assertEqual(stat.S_IMODE(self.path.stat().st_mode), 0o660)
        self.assertEqual(self.path.stat().st_gid, os.getegid())
        with self.assertRaises(ValueError):
            request_unix_worker(self.path, self.signed())
        self.assertEqual(self.runtime.snapshot("agent-one")["remaining"], 10)

    def test_failure_after_bind_before_inode_capture_cleans_socket(self):
        from agent_tool_gateway import LocalToolGateway
        gateway = LocalToolGateway(self.runtime, {"compile-intent": self.issuer})
        real_lstat = os.lstat
        injected = [False]
        def fail_first_bound_target(path, *args, **kwargs):
            if (str(path) == str(self.path) and self.path.exists()
                    and not injected[0]):
                injected[0] = True
                raise OSError("injected filesystem failure")
            return real_lstat(path, *args, **kwargs)
        with patch("agent_secure_worker.os.lstat", side_effect=fail_first_bound_target):
            with self.assertRaises(OSError):
                serve_unix_worker(gateway, self.path, allowed_uids={os.geteuid()},
                                  stop_event=self.stop)
        self.assertTrue(injected[0])
        self.assertFalse(self.path.exists(), "failed bind must not strand a stale socket")

    def test_existing_socket_path_not_overwritten(self):
        self.path.write_text("not a socket")
        with self.assertRaises(ValueError):
            serve_unix_worker(None, self.path, allowed_uids={os.getuid()},
                              ready_event=self.ready, stop_event=self.stop)
        self.assertEqual(self.path.read_text(), "not a socket")


if __name__ == "__main__":
    unittest.main()
