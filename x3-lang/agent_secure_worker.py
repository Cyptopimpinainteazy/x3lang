"""Linux-only local IPC for a trusted Agent Law gateway (development prototype).

A separate worker process owns the Runtime, SQLite DB, signing keys and its
allowlisted tool adapter. A Unix socket is the only interface this module
exposes. It does not create process isolation on its own: run the worker under
an isolated OS user/sandbox and deny agents access to its files and process.
"""
from __future__ import annotations

import os
from pathlib import Path
import socket
import stat
import struct

from agent_tool_gateway import LocalToolGateway, MAX_REQUEST_BYTES, MAX_REPLY_BYTES

MAX_WIRE_BYTES = MAX_REQUEST_BYTES
HEADER = struct.Struct("!I")
RESPONSE = struct.Struct("!BI")
MAX_FAILURE_LENGTH = 32
DENIED = b"request denied"
TIMEOUT = 2.0


def _recv_exact(stream: socket.socket, count: int) -> bytes:
    if not isinstance(count, int) or count < 0 or count > MAX_WIRE_BYTES:
        raise ValueError("invalid frame size")
    chunks = []
    remaining = count
    while remaining:
        block = stream.recv(min(65536, remaining))
        if not block:
            raise ValueError("incomplete frame")
        chunks.append(block)
        remaining -= len(block)
    return b"".join(chunks)


def _send_response(stream: socket.socket, status: int, payload: bytes) -> None:
    if status not in (0, 1) or not isinstance(payload, bytes):
        raise ValueError("invalid response")
    if len(payload) > MAX_REPLY_BYTES:
        raise ValueError("response too large")
    stream.sendall(RESPONSE.pack(status, len(payload)) + payload)


def _check_socket_path(socket_path: str | os.PathLike) -> Path:
    if os.name != "posix" or not hasattr(socket, "SO_PEERCRED"):
        raise RuntimeError("Linux peer credentials required")
    target = Path(socket_path)
    if not target.is_absolute():
        raise ValueError("absolute socket path required")
    parent = target.parent
    pstat = os.lstat(parent)
    if (not stat.S_ISDIR(pstat.st_mode) or pstat.st_uid != os.geteuid()
            or stat.S_IMODE(pstat.st_mode) & 0o077):
        raise ValueError("socket directory must be private and owned by worker")
    if os.path.lexists(target):
        raise ValueError("socket path already exists; will not overwrite")
    if len(os.fsencode(target)) >= 100:
        raise ValueError("socket path too long")
    return target


def _handle_connection(conn: socket.socket, gateway: LocalToolGateway,
                       allowed_uids: frozenset[int]) -> None:
    conn.settimeout(TIMEOUT)
    try:
        raw_creds = conn.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED,
                                   struct.calcsize("3i"))
        _pid, uid, _gid = struct.unpack("3i", raw_creds)
        if uid not in allowed_uids:
            raise ValueError("peer UID is not allowed")
        size = HEADER.unpack(_recv_exact(conn, HEADER.size))[0]
        if not 0 < size <= MAX_WIRE_BYTES:
            raise ValueError("invalid frame length")
        payload = _recv_exact(conn, size)
        result = gateway.submit(payload)
        if not isinstance(result, bytes) or len(result) > MAX_REPLY_BYTES:
            raise ValueError("invalid worker result")
        _send_response(conn, 0, result)
    except (Exception,) as exc:
        # No exception messages, signing data, or server state disclosed to agent.
        try:
            _send_response(conn, 1, DENIED)
        except (OSError, ValueError):
            pass


def serve_unix_worker(gateway: LocalToolGateway,
                      socket_path: str | os.PathLike,
                      *, allowed_uids, ready_event=None, stop_event=None) -> None:
    """Serve sequential one-shot requests from authenticated local peers.

    The process launching this function is trusted. The caller controls OS
    user, sandbox/namespace, filesystem access, and secure key provisioning.
    """
    target = _check_socket_path(socket_path)
    if not isinstance(gateway, LocalToolGateway):
        raise ValueError("trusted LocalToolGateway is required")
    if (not isinstance(allowed_uids, (set, frozenset)) or not allowed_uids
            or any(type(uid) is not int or uid < 0 for uid in allowed_uids)):
        raise ValueError("explicit allowed UID set required")
    allowed = frozenset(allowed_uids)
    if stop_event is None:
        raise ValueError("explicit stop event required in local test server")
    listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    bound = False
    original_inode = None
    try:
        listener.bind(str(target))
        bound = True
        os.chmod(target, 0o600)
        original_inode = os.lstat(target).st_ino
        listener.listen(8)
        listener.settimeout(0.2)
        if ready_event is not None:
            ready_event.set()
        while not stop_event.is_set():
            try:
                conn, _ = listener.accept()
            except socket.timeout:
                continue
            with conn:
                _handle_connection(conn, gateway, allowed)
    finally:
        listener.close()
        if bound:
            try:
                current = os.lstat(target)
                if stat.S_ISSOCK(current.st_mode) and current.st_ino == original_inode:
                    target.unlink()
            except FileNotFoundError:
                pass


def request_unix_worker(socket_path: str | os.PathLike, request: bytes) -> bytes:
    """Small unprivileged client helper; agent signature is checked by gateway."""
    if not isinstance(request, bytes) or not 0 < len(request) <= MAX_WIRE_BYTES:
        raise ValueError("invalid request size")
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as client:
        client.settimeout(TIMEOUT)
        client.connect(str(socket_path))
        client.sendall(HEADER.pack(len(request)) + request)
        header = _recv_exact(client, RESPONSE.size)
        status, size = RESPONSE.unpack(header)
        if status not in (0, 1) or size > MAX_REPLY_BYTES:
            raise ValueError("invalid worker response")
        body = _recv_exact(client, size)
        if status == 1:
            raise ValueError("worker denied request")
        return body
