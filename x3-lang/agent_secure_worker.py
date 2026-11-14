"""Linux Unix-socket secure worker; implementation follows red tests."""
MAX_WIRE_BYTES = 1024 * 1024 + 4096

def serve_unix_worker(gateway, socket_path, *, allowed_uids, ready_event, stop_event):
    raise NotImplementedError("Unix worker boundary not implemented")

def request_unix_worker(socket_path, request):
    raise NotImplementedError("Unix worker client not implemented")
