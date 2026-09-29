#!/usr/bin/env python3
"""Check a built kernel (terminal_os.wasm) against abi/host-abi.toml.

Usage: scripts/check-abi.py [path/to/terminal_os.wasm]

Fails (exit 1) if the kernel imports a function the contract doesn't list,
lists one with a different signature, is missing an export, or exports one
with a different signature. Contract imports the kernel doesn't use are
reported as warnings (hosts may still provide them).
"""
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DEFAULT_WASM = ROOT / "rust/target/wasm32-unknown-unknown/release/terminal_os.wasm"
VALTYPES = {0x7F: "i32", 0x7E: "i64", 0x7D: "f32", 0x7C: "f64"}


def leb(data, i):
    result = shift = 0
    while True:
        b = data[i]
        i += 1
        result |= (b & 0x7F) << shift
        shift += 7
        if b < 0x80:
            return result, i


def name(data, i):
    n, i = leb(data, i)
    return data[i:i + n].decode(), i + n


def parse(path):
    data = path.read_bytes()
    assert data[:4] == b"\0asm", f"{path} is not a wasm module"
    types, imports, func_types, exports = [], [], [], []
    i = 8
    while i < len(data):
        sid = data[i]
        size, i = leb(data, i + 1)
        end, p = i + size, i
        if sid == 1:  # type
            count, p = leb(data, p)
            for _ in range(count):
                assert data[p] == 0x60
                np, p = leb(data, p + 1)
                params = [VALTYPES[data[p + k]] for k in range(np)]
                p += np
                nr, p = leb(data, p)
                results = [VALTYPES[data[p + k]] for k in range(nr)]
                p += nr
                types.append((params, results))
        elif sid == 2:  # import
            count, p = leb(data, p)
            for _ in range(count):
                mod, p = name(data, p)
                field, p = name(data, p)
                kind = data[p]
                p += 1
                if kind == 0:
                    t, p = leb(data, p)
                    imports.append((mod, field, types[t]))
                elif kind == 1:
                    p += 1
                    flags, p = leb(data, p + 0)
                    _, p = leb(data, p)
                    if flags & 1:
                        _, p = leb(data, p)
                elif kind == 2:
                    flags, p = leb(data, p)
                    _, p = leb(data, p)
                    if flags & 1:
                        _, p = leb(data, p)
                elif kind == 3:
                    p += 2
        elif sid == 3:  # function
            count, p = leb(data, p)
            for _ in range(count):
                t, p = leb(data, p)
                func_types.append(types[t])
        elif sid == 7:  # export
            count, p = leb(data, p)
            for _ in range(count):
                n, p = name(data, p)
                kind = data[p]
                idx, p = leb(data, p + 1)
                exports.append((n, kind, idx))
        i = end
    n_imported_funcs = len(imports)
    export_sigs = {}
    for n, kind, idx in exports:
        if kind == 0:
            export_sigs[n] = func_types[idx - n_imported_funcs]
        elif kind == 2:
            export_sigs[n] = "memory"
    return imports, export_sigs


def main():
    wasm = Path(sys.argv[1]) if len(sys.argv) > 1 else DEFAULT_WASM
    if not wasm.exists():
        print(f"kernel not built: {wasm}\n  build: (cd rust && cargo build --release --target wasm32-unknown-unknown -p terminal-os)")
        return 1
    contract = tomllib.loads((ROOT / "abi/host-abi.toml").read_text())
    imports, exports = parse(wasm)
    errors, warnings = [], []

    used = set()
    for mod, field, (params, results) in imports:
        spec = contract["imports"].get(field)
        used.add(field)
        if mod != "env":
            errors.append(f"import {mod}::{field}: only module 'env' is allowed")
        if spec is None:
            errors.append(f"import {field}{params}->{results} is not in the contract")
        elif spec["params"] != params or spec["results"] != results:
            errors.append(f"import {field}: kernel {params}->{results}, contract {spec['params']}->{spec['results']}")
    for field in contract["imports"]:
        if field not in used:
            warnings.append(f"contract import {field} is not used by this kernel")

    for n, spec in contract["exports"].items():
        got = exports.get(n)
        if got is None:
            errors.append(f"export {n} missing")
        elif spec.get("kind") == "memory":
            if got != "memory":
                errors.append(f"export {n}: expected memory")
        elif got == "memory" or list(got[0]) != spec["params"] or list(got[1]) != spec["results"]:
            errors.append(f"export {n}: kernel {got}, contract {spec['params']}->{spec['results']}")

    for w in warnings:
        print("warning:", w)
    for e in errors:
        print("error:", e)
    print(f"{wasm.name}: {len(imports)} imports, {len(exports)} exports checked against abi/host-abi.toml: "
          + ("OK" if not errors else f"{len(errors)} error(s)"))
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
