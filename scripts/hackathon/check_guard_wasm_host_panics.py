#!/usr/bin/env python3
"""Check whether a NEAR contract WASM reports refusal reasons to the chain.

near-sdk 5.x only calls the `env.panic_utf8` host function when the contract is
compiled with `--cfg near` (what `cargo near build` sets). Without that cfg,
`env::panic_str` and `require!` discard their message and execute a WebAssembly
`unreachable`, so explorers show a bare trap instead of the Guard's reason.

This tool reads only the WASM import section. It needs no Rust toolchain, RPC,
keys or network access.

    --expect present   exit 0 only if the module imports env.panic_utf8
    --expect absent    exit 0 only if it does not (documents the legacy deployed build)
"""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
from pathlib import Path

WASM_MAGIC = b"\0asm"
WASM_VERSION = b"\x01\0\0\0"
IMPORT_SECTION = 2
HOST_MODULE = "env"
HOST_PANIC = "panic_utf8"


class WasmFormatError(ValueError):
    """The input is not a well-formed WebAssembly module (as far as this reader needs)."""


def _read_u32(data: bytes, pos: int) -> tuple[int, int]:
    """Decode an unsigned LEB128 value of at most 32 bits."""
    result = 0
    shift = 0
    for i in range(5):
        if pos >= len(data):
            raise WasmFormatError("truncated LEB128 integer")
        byte = data[pos]
        pos += 1
        result |= (byte & 0x7F) << shift
        if byte & 0x80 == 0:
            if i == 4 and byte & 0x70:
                raise WasmFormatError("LEB128 integer exceeds 32 bits")
            return result, pos
        shift += 7
    raise WasmFormatError("LEB128 integer longer than 5 bytes")


def _read_name(data: bytes, pos: int, end: int) -> tuple[str, int]:
    length, pos = _read_u32(data, pos)
    if pos + length > end:
        raise WasmFormatError("name runs past its section")
    try:
        return data[pos : pos + length].decode("utf-8"), pos + length
    except UnicodeDecodeError as exc:
        raise WasmFormatError("name is not valid UTF-8") from exc


def _skip_limits(data: bytes, pos: int) -> int:
    if pos >= len(data):
        raise WasmFormatError("truncated limits")
    flags = data[pos]
    pos += 1
    if flags & ~0x07:
        raise WasmFormatError(f"unsupported limits flags {flags:#x}")
    _, pos = _read_u32(data, pos)
    if flags & 0x01:
        _, pos = _read_u32(data, pos)
    return pos


def _skip_import_desc(data: bytes, pos: int) -> tuple[int, int]:
    if pos >= len(data):
        raise WasmFormatError("truncated import descriptor")
    kind = data[pos]
    pos += 1
    if kind == 0x00:  # function: type index
        _, pos = _read_u32(data, pos)
    elif kind == 0x01:  # table: reftype + limits
        pos = _skip_limits(data, pos + 1)
    elif kind == 0x02:  # memory: limits
        pos = _skip_limits(data, pos)
    elif kind == 0x03:  # global: valtype + mutability
        pos += 2
    elif kind == 0x04:  # tag: attribute + type index
        _, pos = _read_u32(data, pos + 1)
    else:
        raise WasmFormatError(f"unknown import kind {kind:#x}")
    return kind, pos


def imported_functions(wasm: bytes) -> list[tuple[str, str]]:
    """Return every (module, name) function import, in declaration order."""
    if len(wasm) < 8 or wasm[:4] != WASM_MAGIC:
        raise WasmFormatError("missing WebAssembly magic header")
    if wasm[4:8] != WASM_VERSION:
        raise WasmFormatError("unsupported WebAssembly binary version")
    pos = 8
    imports: list[tuple[str, str]] = []
    seen_import_section = False
    while pos < len(wasm):
        section_id = wasm[pos]
        size, body = _read_u32(wasm, pos + 1)
        end = body + size
        if end > len(wasm):
            raise WasmFormatError(f"section {section_id} runs past end of module")
        if section_id == IMPORT_SECTION:
            if seen_import_section:
                raise WasmFormatError("duplicate import section")
            seen_import_section = True
            count, cur = _read_u32(wasm, body)
            for _ in range(count):
                module, cur = _read_name(wasm, cur, end)
                name, cur = _read_name(wasm, cur, end)
                kind, cur = _skip_import_desc(wasm, cur)
                if cur > end:
                    raise WasmFormatError("import entry runs past its section")
                if kind == 0x00:
                    imports.append((module, name))
            if cur != end:
                raise WasmFormatError("import section has trailing bytes")
        pos = end
    return imports


def inspect(wasm: bytes) -> dict:
    funcs = imported_functions(wasm)
    host = sorted({name for module, name in funcs if module == HOST_MODULE})
    return {
        "schema": "x3.near.wasm-host-panic-check.v1",
        "wasm_bytes": len(wasm),
        "wasm_sha256": hashlib.sha256(wasm).hexdigest(),
        "env_function_imports": len(host),
        "imports_panic_utf8": HOST_PANIC in host,
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("wasm", type=Path)
    parser.add_argument("--expect", choices=("present", "absent"), required=True)
    args = parser.parse_args(argv)
    try:
        report = inspect(args.wasm.read_bytes())
    except (OSError, WasmFormatError) as exc:
        print(f"ERROR: cannot inspect {args.wasm}: {exc}", file=sys.stderr)
        return 2
    print(json.dumps(report, sort_keys=True))
    if report["env_function_imports"] == 0:
        print("ERROR: module imports no NEAR host functions; not a NEAR contract build", file=sys.stderr)
        return 1
    has = report["imports_panic_utf8"]
    if args.expect == "present" and not has:
        print(
            "ERROR: WASM does not import env.panic_utf8. It was built without `--cfg near`, so every "
            "refusal reason is discarded and appears on chain as a bare `unreachable` trap.",
            file=sys.stderr,
        )
        return 1
    if args.expect == "absent" and has:
        print("ERROR: expected the legacy build without env.panic_utf8, but the module imports it", file=sys.stderr)
        return 1
    print(f"OK: env.panic_utf8 {'imported' if has else 'not imported'} as expected ({args.expect})")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
