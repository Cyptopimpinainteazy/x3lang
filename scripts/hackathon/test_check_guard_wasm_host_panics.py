"""Tests for check_guard_wasm_host_panics.py using hand-assembled WASM modules."""

from __future__ import annotations

import contextlib
import io
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import check_guard_wasm_host_panics as chk  # noqa: E402

HEADER = b"\0asm\x01\0\0\0"
# Type section: one function type () -> ()
TYPE_SECTION = bytes([0x01, 0x04, 0x01, 0x60, 0x00, 0x00])


def leb(n: int) -> bytes:
    out = bytearray()
    while True:
        byte = n & 0x7F
        n >>= 7
        if n:
            out.append(byte | 0x80)
        else:
            out.append(byte)
            return bytes(out)


def name(text: str) -> bytes:
    raw = text.encode()
    return leb(len(raw)) + raw


def section(section_id: int, body: bytes) -> bytes:
    return bytes([section_id]) + leb(len(body)) + body


def func_import(module: str, field: str) -> bytes:
    return name(module) + name(field) + b"\x00" + leb(0)


def memory_import(module: str, field: str) -> bytes:
    return name(module) + name(field) + b"\x02" + b"\x01" + leb(1) + leb(16)


def module(*entries: bytes, extra: bytes = b"") -> bytes:
    imports = leb(len(entries)) + b"".join(entries)
    return HEADER + TYPE_SECTION + section(2, imports) + extra


def run(wasm: bytes, expect: str) -> tuple[int, str, str]:
    with tempfile.TemporaryDirectory() as tmp:
        path = Path(tmp) / "c.wasm"
        path.write_bytes(wasm)
        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            code = chk.main([str(path), "--expect", expect])
    return code, out.getvalue(), err.getvalue()


NEAR_BUILD = module(
    func_import("env", "storage_usage"),
    func_import("env", "panic_utf8"),
    memory_import("env", "memory"),
    extra=section(0, name("custom") + b"payload"),
)
FALLBACK_BUILD = module(func_import("env", "storage_usage"), func_import("env", "input"))


class HostPanicImportTest(unittest.TestCase):
    def test_cfg_near_build_passes_present(self) -> None:
        code, out, _ = run(NEAR_BUILD, "present")
        self.assertEqual(code, 0)
        self.assertIn('"imports_panic_utf8": true', out)

    def test_fallback_build_fails_present_with_reason(self) -> None:
        code, _, err = run(FALLBACK_BUILD, "present")
        self.assertEqual(code, 1)
        self.assertIn("--cfg near", err)
        self.assertIn("unreachable", err)

    def test_legacy_expectation_is_exact(self) -> None:
        self.assertEqual(run(FALLBACK_BUILD, "absent")[0], 0)
        self.assertEqual(run(NEAR_BUILD, "absent")[0], 1)

    def test_panic_utf8_from_another_module_does_not_count(self) -> None:
        wasm = module(func_import("env", "input"), func_import("wasi", "panic_utf8"))
        self.assertEqual(run(wasm, "present")[0], 1)

    def test_module_without_host_imports_is_rejected_both_ways(self) -> None:
        wasm = HEADER + TYPE_SECTION
        for expect in ("present", "absent"):
            code, _, err = run(wasm, expect)
            self.assertEqual(code, 1, expect)
            self.assertIn("not a NEAR contract", err)

    def test_malformed_inputs_exit_2(self) -> None:
        cases = {
            "not wasm": b"PK\x03\x04",
            "bad version": b"\0asm\x02\0\0\0",
            "section past end": HEADER + bytes([0x02, 0x7F, 0x00]),
            "name past section": HEADER + section(2, leb(1) + leb(50) + b"env"),
            "unknown import kind": HEADER + section(2, leb(1) + name("env") + name("x") + b"\x09"),
            "trailing import bytes": HEADER + section(2, leb(1) + func_import("env", "input") + b"\x00"),
            "duplicate import section": NEAR_BUILD + section(2, leb(0)),
            "overlong leb": HEADER + bytes([0x02, 0x80, 0x80, 0x80, 0x80, 0x80, 0x01]),
            "invalid utf8 name": HEADER + section(2, leb(1) + leb(2) + b"\xff\xfe" + name("x") + b"\x00\x00"),
        }
        for label, wasm in cases.items():
            with self.subTest(label):
                code, _, err = run(wasm, "present")
                self.assertEqual(code, 2, err)
                self.assertIn("cannot inspect", err)

    def test_missing_file_exits_2(self) -> None:
        err = io.StringIO()
        with contextlib.redirect_stderr(err), contextlib.redirect_stdout(io.StringIO()):
            code = chk.main(["/nonexistent/x3_guard.wasm", "--expect", "present"])
        self.assertEqual(code, 2)

    def test_report_hash_matches_bytes(self) -> None:
        report = chk.inspect(NEAR_BUILD)
        import hashlib

        self.assertEqual(report["wasm_sha256"], hashlib.sha256(NEAR_BUILD).hexdigest())
        self.assertEqual(report["wasm_bytes"], len(NEAR_BUILD))
        self.assertEqual(report["env_function_imports"], 2)


if __name__ == "__main__":
    unittest.main()
