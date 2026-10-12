#!/usr/bin/env python3
"""Generate the UI protocol codecs and shared test vectors from abi/ui-protocol.toml.

Usage: scripts/gen-ui-proto.py [--check]

Writes
  rust/crates/ecm-ui-proto/src/generated.rs
  src/main/java/com/example/evanscomputermod/computer/ui/proto/UiProto.java
  abi/ui-test-vectors/valid.txt
  abi/ui-test-vectors/invalid.txt

The vectors are encoded by an independent Python implementation of the wire
format (below), from sample values derived from the contract plus hand-written
cases, and every one is checked by the Python decoder before it is written.
--check exits 1 if any output differs from what is on disk (CI / pre-commit).
"""
import re
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SPEC = ROOT / "abi/ui-protocol.toml"
RUST_OUT = ROOT / "rust/crates/ecm-ui-proto/src/generated.rs"
JAVA_OUT = ROOT / "src/main/java/com/example/evanscomputermod/computer/ui/proto/UiProto.java"
VEC_DIR = ROOT / "abi/ui-test-vectors"

PRIMS = {"u8": 1, "u16": 2, "u32": 4, "u64": 8, "i16": 2, "i32": 4, "bool": 1, "str": 4, "bytes": 4}
INT_BOUNDS = {"u8": (0, 0xFF), "u16": (0, 0xFFFF), "u32": (0, 0xFFFFFFFF), "u64": (0, 2**64 - 1),
              "i16": (-0x8000, 0x7FFF), "i32": (-2**31, 2**31 - 1)}
DEFAULT_STR_MAX = 4096
DEFAULT_ARR_MAX = 65535


# ============================================================== model

class T:
    """A type reference: prim, arr, enum, flags, struct or union."""

    def __init__(self, kind, name, elem=None):
        self.kind, self.name, self.elem = kind, name, elem

    def __repr__(self):
        return f"[{self.elem!r}]" if self.kind == "arr" else self.name


class Field:
    def __init__(self, name, t, cons):
        self.name, self.t = name, t
        self.max = cons.get("max")
        self.itemmax = cons.get("itemmax")
        self.lo, self.hi = cons.get("lo"), cons.get("hi")
        self.choices = cons.get("in")
        self.sorted = cons.get("sorted", False)


class Spec:
    def __init__(self, raw):
        self.version = raw["version"]
        self.max_message = raw["max_message"]
        self.enums = {}
        for n, e in raw.get("enums", {}).items():
            self.enums[n] = {"type": e["type"], "values": list(e["values"].items()), "doc": e.get("doc", "")}
        self.flags = {}
        for n, f in raw.get("flags", {}).items():
            bits = list(f["bits"].items())
            mask = 0
            for _, b in bits:
                mask |= b
            self.flags[n] = {"type": f["type"], "bits": bits, "mask": mask}
        self.structs = {}
        self.unions = {}
        self.messages = {}
        # names must be known before field types are parsed
        names = set(self.enums) | set(self.flags) | set(raw.get("structs", {})) | set(raw.get("unions", {}))
        self.names = names
        for n, s in raw.get("structs", {}).items():
            self.structs[n] = {"fields": [self.field(x) for x in s["fields"]], "doc": s.get("doc", "")}
        for n, u in raw.get("unions", {}).items():
            variants = []
            for vn, v in u["variants"].items():
                variants.append({"name": vn, "tag": v["tag"], "fields": [self.field(x) for x in v["fields"]],
                                 "validate": v.get("validate"), "doc": v.get("doc", "")})
            self.unions[n] = {"tag": u["tag"], "variants": variants, "doc": u.get("doc", "")}
        self.node_props = {k: v for k, v in raw.get("node_props", {}).items() if k != "doc"}
        opcodes = {}
        for n, m in raw["messages"].items():
            op = m["opcode"]
            assert op not in opcodes, f"opcode {op:#06x} used by {opcodes.get(op)} and {n}"
            opcodes[op] = n
            self.messages[n] = {"opcode": op, "dir": m["dir"], "doc": m.get("doc", ""),
                                "fields": [self.field(x) for x in m["fields"]]}
        # node_props must reference real Prop variants and NodeKind values
        prop_names = {v["name"] for v in self.unions["Prop"]["variants"]}
        kinds = {k for k, _ in self.enums["NodeKind"]["values"]}
        for k, props in self.node_props.items():
            assert k in kinds, k
            for p in props:
                assert p in prop_names, p

    def parse_type(self, s):
        if s.startswith("["):
            assert s.endswith("]"), s
            return T("arr", None, self.parse_type(s[1:-1]))
        if s in PRIMS:
            return T("prim", s)
        if s in getattr(self, "enums", {}):
            return T("enum", s)
        if s in getattr(self, "flags", {}):
            return T("flags", s)
        if s in self.names:
            # struct or union; resolved lazily
            return T("named", s)
        raise ValueError(f"unknown type {s!r}")

    def field(self, text):
        m = re.fullmatch(r"\s*(\w+)\s*:\s*(\S+)\s*(.*)", text)
        assert m, text
        name, ty, rest = m.groups()
        cons = {}
        for tok in rest.split():
            if tok == "sorted":
                cons["sorted"] = True
            elif tok.startswith("max="):
                cons["max"] = int(tok[4:], 0)
            elif tok.startswith("itemmax="):
                cons["itemmax"] = int(tok[8:], 0)
            elif tok.startswith("range="):
                lo, hi = tok[6:].split("..")
                if lo:
                    cons["lo"] = int(lo, 0)
                if hi:
                    cons["hi"] = int(hi, 0)
            elif tok.startswith("in="):
                cons["in"] = [int(x, 0) for x in tok[3:].split(",")]
            else:
                raise ValueError(f"bad constraint {tok!r} in {text!r}")
        return Field(name, self.parse_type(ty), cons)

    def kind_of(self, t):
        if t.kind == "named":
            return "struct" if t.name in self.structs else "union"
        return t.kind

    def underlying(self, t):
        k = self.kind_of(t)
        if k == "enum":
            return self.enums[t.name]["type"]
        if k == "flags":
            return self.flags[t.name]["type"]
        return t.name

    def min_size(self, t):
        k = self.kind_of(t)
        if k == "prim":
            return PRIMS[t.name]
        if k in ("enum", "flags"):
            return PRIMS[self.underlying(t)]
        if k == "arr":
            return 4
        if k == "struct":
            return sum(self.min_size(f.t) for f in self.structs[t.name]["fields"])
        if k == "union":
            u = self.unions[t.name]
            return PRIMS[u["tag"]] + min(sum(self.min_size(f.t) for f in v["fields"]) for v in u["variants"])
        raise AssertionError(t)


def camel(s):
    return "".join(p[:1].upper() + p[1:] for p in s.split("_"))


def lower_camel(s):
    c = camel(s)
    return c[:1].lower() + c[1:]


def upper_snake(s):
    return re.sub(r"(?<=[a-z0-9])(?=[A-Z])", "_", s).upper()


def snake(s):
    return re.sub(r"(?<=[a-z0-9])(?=[A-Z])", "_", s).lower()


# ============================================================== python reference codec

class Bad(Exception):
    pass


class W:
    def __init__(self):
        self.b = bytearray()

    def int(self, ty, v):
        lo, hi = INT_BOUNDS[ty]
        assert lo <= v <= hi, (ty, v)
        self.b += v.to_bytes(PRIMS[ty], "little", signed=ty.startswith("i"))


def py_enc(spec, t, v, w):
    k = spec.kind_of(t)
    if k == "prim":
        if t.name == "bool":
            w.b.append(1 if v else 0)
        elif t.name == "str":
            raw = v.encode("utf-8")
            w.int("u32", len(raw))
            w.b += raw
        elif t.name == "bytes":
            w.int("u32", len(v))
            w.b += v
        else:
            w.int(t.name, v)
    elif k == "enum":
        w.int(spec.underlying(t), dict(spec.enums[t.name]["values"])[v])
    elif k == "flags":
        w.int(spec.underlying(t), v)
    elif k == "arr":
        w.int("u32", len(v))
        for x in v:
            py_enc(spec, t.elem, x, w)
    elif k == "struct":
        for f in spec.structs[t.name]["fields"]:
            py_enc(spec, f.t, v[f.name], w)
    elif k == "union":
        u = spec.unions[t.name]
        var = next(x for x in u["variants"] if x["name"] == v["type"])
        w.int(u["tag"], var["tag"])
        for f in var["fields"]:
            py_enc(spec, f.t, v[f.name], w)


def py_encode_message(spec, name, value, flags=0):
    w = W()
    for f in spec.messages[name]["fields"]:
        py_enc(spec, f.t, value[f.name], w)
    body = bytes(w.b)
    return spec.messages[name]["opcode"].to_bytes(2, "little") + flags.to_bytes(2, "little") + \
        len(body).to_bytes(4, "little") + body


class R:
    def __init__(self, b):
        self.b, self.p = b, 0

    def take(self, n):
        if self.p + n > len(self.b):
            raise Bad("truncated")
        out = self.b[self.p:self.p + n]
        self.p += n
        return out

    def int(self, ty):
        return int.from_bytes(self.take(PRIMS[ty]), "little", signed=ty.startswith("i"))


def check_int(f, v, path):
    if f is None:
        return
    if f.lo is not None and v < f.lo or f.hi is not None and v > f.hi:
        raise Bad(f"range {path}")
    if f.choices is not None and v not in f.choices:
        raise Bad(f"in {path}")


def py_dec(spec, t, f, r, path):
    k = spec.kind_of(t)
    if k == "prim":
        if t.name == "bool":
            b = r.int("u8")
            if b > 1:
                raise Bad(f"bool {path}")
            return b == 1
        if t.name in ("str", "bytes"):
            n = r.int("u32")
            limit = (f.max if f and f.max is not None else
                     (DEFAULT_STR_MAX if t.name == "str" else spec.max_message))
            if n > limit:
                raise Bad(f"too long {path}")
            raw = r.take(n)
            if t.name == "bytes":
                return bytes(raw)
            try:
                return bytes(raw).decode("utf-8")
            except UnicodeDecodeError:
                raise Bad(f"utf8 {path}")
        v = r.int(t.name)
        check_int(f, v, path)
        return v
    if k == "enum":
        code = r.int(spec.underlying(t))
        for n, c in spec.enums[t.name]["values"]:
            if c == code:
                return n
        raise Bad(f"enum {path}")
    if k == "flags":
        v = r.int(spec.underlying(t))
        if v & ~spec.flags[t.name]["mask"]:
            raise Bad(f"flags {path}")
        return v
    if k == "arr":
        n = r.int("u32")
        limit = f.max if f and f.max is not None else DEFAULT_ARR_MAX
        if n > limit or n * spec.min_size(t.elem) > len(r.b) - r.p:
            raise Bad(f"count {path}")
        item = Field("item", t.elem, {"max": f.itemmax} if f and f.itemmax else {})
        out = [py_dec(spec, t.elem, item, r, path) for _ in range(n)]
        if f and f.sorted:
            keys = [sort_key(spec, t.elem, x) for x in out]
            for a, b in zip(keys, keys[1:]):
                if a >= b:
                    raise Bad(f"sorted {path}")
        return out
    if k == "struct":
        return {g.name: py_dec(spec, g.t, g, r, f"{path}.{g.name}") for g in spec.structs[t.name]["fields"]}
    if k == "union":
        u = spec.unions[t.name]
        tag = r.int(u["tag"])
        var = next((x for x in u["variants"] if x["tag"] == tag), None)
        if var is None:
            raise Bad(f"union tag {path}")
        v = {"type": var["name"]}
        for g in var["fields"]:
            v[g.name] = py_dec(spec, g.t, g, r, f"{path}.{g.name}")
        if var["validate"] == "node_props":
            allowed = spec.node_props[v["kind"]]
            for p in v["props"]:
                if p["type"] not in allowed:
                    raise Bad(f"prop {p['type']} not allowed on {v['kind']}")
        return v
    raise AssertionError(t)


def sort_key(spec, t, v):
    k = spec.kind_of(t)
    if k == "union":
        u = spec.unions[t.name]
        return next(x["tag"] for x in u["variants"] if x["name"] == v["type"])
    if k == "struct":
        first = spec.structs[t.name]["fields"][0]
        x = v[first.name]
        return x.encode("utf-8") if isinstance(x, str) else x
    raise AssertionError("sorted needs a union or struct array")


def py_decode(spec, buf):
    """Decode exactly one whole message; raise Bad on anything wrong."""
    if len(buf) < 8:
        raise Bad("incomplete header")
    op, flags, n = int.from_bytes(buf[0:2], "little"), int.from_bytes(buf[2:4], "little"), \
        int.from_bytes(buf[4:8], "little")
    if flags:
        raise Bad("flags")
    name = next((m for m, d in spec.messages.items() if d["opcode"] == op), None)
    if name is None:
        raise Bad("unknown opcode")
    if n > spec.max_message:
        raise Bad("too large")
    if len(buf) != 8 + n:
        raise Bad("length mismatch")
    r = R(buf[8:])
    v = {f.name: py_dec(spec, f.t, f, r, f"{name}.{f.name}") for f in spec.messages[name]["fields"]}
    if r.p != len(r.b):
        raise Bad("trailing bytes")
    return name, v


def json_str(s):
    out = ['"']
    for ch in s:
        if ch == '"':
            out.append('\\"')
        elif ch == "\\":
            out.append("\\\\")
        elif ord(ch) < 0x20:
            out.append("\\u%04x" % ord(ch))
        else:
            out.append(ch)
    out.append('"')
    return "".join(out)


def py_json(spec, t, v):
    k = spec.kind_of(t)
    if k == "prim":
        if t.name == "bool":
            return "true" if v else "false"
        if t.name == "str":
            return json_str(v)
        if t.name == "bytes":
            return '"' + v.hex() + '"'
        return str(v)
    if k == "enum":
        return json_str(v)
    if k == "flags":
        return str(v)
    if k == "arr":
        return "[" + ",".join(py_json(spec, t.elem, x) for x in v) + "]"
    if k == "struct":
        return "{" + ",".join(f'"{f.name}":' + py_json(spec, f.t, v[f.name])
                              for f in spec.structs[t.name]["fields"]) + "}"
    if k == "union":
        var = next(x for x in spec.unions[t.name]["variants"] if x["name"] == v["type"])
        parts = [f'"type":"{var["name"]}"'] + [f'"{f.name}":' + py_json(spec, f.t, v[f.name]) for f in var["fields"]]
        return "{" + ",".join(parts) + "}"
    raise AssertionError(t)


def py_json_message(spec, name, v):
    parts = [f'"op":"{name}"'] + [f'"{f.name}":' + py_json(spec, f.t, v[f.name])
                                  for f in spec.messages[name]["fields"]]
    return "{" + ",".join(parts) + "}"


# ============================================================== Rust codegen

def rs_prim(name):
    return {"str": "String", "bytes": "Vec<u8>"}.get(name, name)


def rs_type(spec, t):
    k = spec.kind_of(t)
    if k == "prim":
        return rs_prim(t.name)
    if k == "flags":
        return spec.underlying(t)
    if k == "arr":
        return f"Vec<{rs_type(spec, t.elem)}>"
    return t.name


def rs_enc(spec, t, e, depth=0):
    """Statements encoding the value referenced by expression `e` (a reference)."""
    k = spec.kind_of(t)
    if k == "prim":
        if t.name == "str":
            return [f"w.str({e});"]
        if t.name == "bytes":
            return [f"w.bytes({e});"]
        return [f"w.{t.name}(*{e});"]
    if k == "enum":
        return [f"w.{spec.underlying(t)}(({e}).code());"]
    if k == "flags":
        return [f"w.{spec.underlying(t)}(*{e});"]
    if k == "arr":
        x = f"x{depth}"
        inner = rs_enc(spec, t.elem, x, depth + 1)
        return [f"w.u32(({e}).len() as u32);", f"for {x} in ({e}).iter() {{"] + ["    " + s for s in inner] + ["}"]
    return [f"({e}).enc(w);"]


def rs_check_int(f, var, path):
    conds = []
    if f is not None:
        if f.lo is not None:
            conds.append(f"{var} < {f.lo}")
        if f.hi is not None:
            conds.append(f"{var} > {f.hi}")
        if f.choices is not None:
            conds.append("!matches!(" + var + ", " + " | ".join(str(c) for c in f.choices) + ")")
    if not conds:
        return []
    return [f"if {' || '.join(conds)} {{ return Err(DecodeError::OutOfRange(\"{path}\")); }}"]


def rs_dec(spec, t, f, path, depth=0):
    """An expression decoding one value from reader `r`."""
    k = spec.kind_of(t)
    if k == "prim":
        if t.name == "str":
            limit = f.max if f and f.max is not None else DEFAULT_STR_MAX
            return f'r.str({limit}, "{path}")?'
        if t.name == "bytes":
            limit = f.max if f and f.max is not None else spec.max_message
            return f'r.bytes({limit}, "{path}")?'
        if t.name == "bool":
            return f'r.bool("{path}")?'
        checks = rs_check_int(f, "v", path)
        if not checks:
            return f"r.{t.name}()?"
        return "{ let v = r." + t.name + "()?; " + " ".join(checks) + " v }"
    if k == "enum":
        return f'{t.name}::from_code(r.{spec.underlying(t)}()?).ok_or(DecodeError::BadEnum("{path}"))?'
    if k == "flags":
        mask = spec.flags[t.name]["mask"]
        return ("{ let v = r." + spec.underlying(t) + "()?; if v & !" + hex(mask) +
                f' != 0 {{ return Err(DecodeError::BadFlags("{path}")); }} v }}')
    if k == "arr":
        limit = f.max if f and f.max is not None else DEFAULT_ARR_MAX
        item = Field("item", t.elem, {"max": f.itemmax} if f and f.itemmax else {})
        inner = rs_dec(spec, t.elem, item, path, depth + 1)
        n, v = f"n{depth}", f"v{depth}"
        s = (f"{{ let {n} = r.count({limit}, {spec.min_size(t.elem)}, \"{path}\")?; "
             f"let mut {v} = Vec::with_capacity({n}); for _ in 0..{n} {{ {v}.push({inner}); }} ")
        if f and f.sorted:
            ek = spec.kind_of(t.elem)
            if ek == "union":
                key = "tag()"
            else:
                first = spec.structs[t.elem.name]["fields"][0]
                key = first.name + (".as_bytes()" if first.t.name == "str" else "")
            s += (f"for i in 1..{v}.len() {{ if {v}[i - 1].{key} >= {v}[i].{key} "
                  f"{{ return Err(DecodeError::Unsorted(\"{path}\")); }} }} ")
        return s + v + " }"
    return f"{t.name}::dec(r)?"


def rs_json(spec, t, e, depth=0):
    """Statements appending the JSON of the value referenced by `e` to `o`."""
    k = spec.kind_of(t)
    if k == "prim":
        if t.name == "str":
            return [f"json_str(o, {e});"]
        if t.name == "bytes":
            return [f"json_hex(o, {e});"]
        if t.name == "bool":
            return [f'o.push_str(if *{e} {{ "true" }} else {{ "false" }});']
        if t.name.startswith("u"):
            return [f"json_u(o, *{e} as u64);"]
        return [f"json_i(o, *{e} as i64);"]
    if k == "enum":
        return [f"json_str(o, ({e}).name());"]
    if k == "flags":
        return [f"json_u(o, *{e} as u64);"]
    if k == "arr":
        x, i = f"x{depth}", f"i{depth}"
        inner = rs_json(spec, t.elem, x, depth + 1)
        return ["o.push('[');", f"for ({i}, {x}) in ({e}).iter().enumerate() {{",
                f"    if {i} > 0 {{ o.push(','); }}"] + ["    " + s for s in inner] + ["}", "o.push(']');"]
    return [f"({e}).json(o);"]


def rs_doc(doc, ind=""):
    return [f"{ind}/// {line}" for line in doc.splitlines()] if doc else []


def rs_fields_json(spec, fields, access, first_prefix):
    out = []
    for n, f in enumerate(fields):
        sep = "" if n == 0 and not first_prefix else ","
        out.append(f'o.push_str("{sep}\\"{f.name}\\":");')
        out += rs_json(spec, f.t, access(f))
    return out


def gen_rust(spec):
    L = []
    a = L.append
    a("// @generated by scripts/gen-ui-proto.py from abi/ui-protocol.toml. Do not edit.")
    a("#![allow(clippy::all, unused_comparisons, unused_parens)]")
    a("")
    a("use crate::wire::{json_hex, json_i, json_str, json_u, DecodeError, Reader, Writer};")
    a("use alloc::string::String;")
    a("use alloc::vec::Vec;")
    a("")
    a(f"/// Protocol version sent in Hello / Welcome.")
    a(f"pub const VERSION: u16 = {spec.version};")
    a(f"/// Largest message body accepted, in bytes.")
    a(f"pub const MAX_MESSAGE: u32 = {spec.max_message};")
    a("")
    a("/// Opcode of every message.")
    a("pub mod opcode {")
    for n, m in spec.messages.items():
        a(f"    pub const {upper_snake(n)}: u16 = {m['opcode']:#06x};")
    a("}")
    a("")
    # enums
    for n, e in spec.enums.items():
        ut = e["type"]
        a(rs_doc(e["doc"]) and "\n".join(rs_doc(e["doc"])) or f"/// `{n}` ({ut}).")
        a("#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]")
        a(f"#[repr({ut})]")
        a(f"pub enum {n} {{")
        for i, (vn, code) in enumerate(e["values"]):
            if i == 0:
                a("    #[default]")
            a(f"    {camel(vn)} = {code},")
        a("}")
        a("")
        a(f"impl {n} {{")
        a(f"    pub const ALL: &'static [{n}] = &[" + ", ".join(f"{n}::{camel(v)}" for v, _ in e["values"]) + "];")
        a(f"    pub fn code(self) -> {ut} {{ self as {ut} }}")
        a(f"    pub fn from_code(code: {ut}) -> Option<Self> {{")
        a("        match code {")
        for vn, code in e["values"]:
            a(f"            {code} => Some({n}::{camel(vn)}),")
        a("            _ => None,")
        a("        }")
        a("    }")
        a("    /// The name used in the contract and in test vectors.")
        a("    pub fn name(self) -> &'static str {")
        a("        match self {")
        for vn, _ in e["values"]:
            a(f'            {n}::{camel(vn)} => "{vn}",')
        a("        }")
        a("    }")
        a(f"    pub fn from_name(name: &str) -> Option<Self> {{")
        a("        Self::ALL.iter().copied().find(|v| v.name() == name)")
        a("    }")
        a("}")
        a("")
    # flags
    for n, f in spec.flags.items():
        a(f"/// Bits of `{n}` ({f['type']}).")
        a(f"pub mod {snake(n)} {{")
        for bn, b in f["bits"]:
            a(f"    pub const {bn.upper()}: {f['type']} = {b:#x};")
        a(f"    pub const MASK: {f['type']} = {f['mask']:#x};")
        a("}")
        a("")
    # structs
    for n, s in spec.structs.items():
        L.extend(rs_doc(s["doc"]))
        a("#[derive(Clone, Debug, PartialEq, Eq, Default)]")
        a(f"pub struct {n} {{")
        for f in s["fields"]:
            a(f"    pub {f.name}: {rs_type(spec, f.t)},")
        a("}")
        a("")
        a(f"impl {n} {{")
        a("    pub fn enc(&self, w: &mut Writer) {")
        for f in s["fields"]:
            for st in rs_enc(spec, f.t, f"&self.{f.name}"):
                a("        " + st)
        a("    }")
        a("    pub fn dec(r: &mut Reader) -> Result<Self, DecodeError> {")
        for f in s["fields"]:
            a(f"        let {f.name} = {rs_dec(spec, f.t, f, f'{n}.{f.name}')};")
        a(f"        Ok({n} {{ " + ", ".join(f.name for f in s["fields"]) + " })")
        a("    }")
        a("    pub fn json(&self, o: &mut String) {")
        a("        o.push('{');")
        for st in rs_fields_json(spec, s["fields"], lambda f: f"&self.{f.name}", False):
            a("        " + st)
        a("        o.push('}');")
        a("    }")
        a("}")
        a("")
    # unions
    for n, u in spec.unions.items():
        ut = u["tag"]
        L.extend(rs_doc(u["doc"]))
        a("#[derive(Clone, Debug, PartialEq, Eq)]")
        a(f"pub enum {n} {{")
        for v in u["variants"]:
            L.extend(rs_doc(v["doc"], "    "))
            if len(v["fields"]) == 1 and v["fields"][0].name == "value":
                a(f"    {v['name']}({rs_type(spec, v['fields'][0].t)}),")
            else:
                a(f"    {v['name']} {{ " + ", ".join(f"{f.name}: {rs_type(spec, f.t)}" for f in v["fields"]) + " },")
        a("}")
        a("")
        a(f"impl {n} {{")
        a(f"    pub fn tag(&self) -> {ut} {{")
        a("        match self {")
        for v in u["variants"]:
            pat = f"{n}::{v['name']}(..)" if is_tuple(v) else f"{n}::{v['name']} {{ .. }}"
            a(f"            {pat} => {v['tag']},")
        a("        }")
        a("    }")
        a(f"    pub fn name(&self) -> &'static str {{")
        a("        match self {")
        for v in u["variants"]:
            pat = f"{n}::{v['name']}(..)" if is_tuple(v) else f"{n}::{v['name']} {{ .. }}"
            a(f'            {pat} => "{v["name"]}",')
        a("        }")
        a("    }")
        a("    pub fn enc(&self, w: &mut Writer) {")
        a(f"        w.{ut}(self.tag());")
        a("        match self {")
        for v in u["variants"]:
            a(f"            {rs_pattern(n, v)} => {{")
            for f in v["fields"]:
                for st in rs_enc(spec, f.t, f.name):
                    a("                " + st)
            a("            }")
        a("        }")
        a("    }")
        a("    pub fn dec(r: &mut Reader) -> Result<Self, DecodeError> {")
        a(f"        Ok(match r.{ut}()? {{")
        for v in u["variants"]:
            a(f"            {v['tag']} => {{")
            for f in v["fields"]:
                a(f"                let {f.name} = {rs_dec(spec, f.t, f, f'{n}.{v["name"]}.{f.name}')};")
            if v["validate"] == "node_props":
                a("                for p in props.iter() {")
                a("                    if !node_allows(kind, p.tag()) {")
                a(f'                        return Err(DecodeError::PropNotAllowed("{n}.{v["name"]}.props"));')
                a("                    }")
                a("                }")
            if is_tuple(v):
                a(f"                {n}::{v['name']}(value)")
            else:
                a(f"                {n}::{v['name']} {{ " + ", ".join(f.name for f in v["fields"]) + " }")
            a("            }")
        a(f'            _ => return Err(DecodeError::BadEnum("{n}")),')
        a("        })")
        a("    }")
        a("    pub fn json(&self, o: &mut String) {")
        a("        match self {")
        for v in u["variants"]:
            a(f"            {rs_pattern(n, v)} => {{")
            a(f'                o.push_str("{{\\"type\\":\\"{v["name"]}\\"");')
            for st in rs_fields_json(spec, v["fields"], lambda f: f.name, True):
                a("                " + st)
            a("                o.push('}');")
            a("            }")
        a("        }")
        a("    }")
        a("}")
        a("")
    # node props table
    if spec.node_props:
        tags = {v["name"]: v["tag"] for v in spec.unions["Prop"]["variants"]}
        a("/// Whether a node of `kind` accepts the prop with tag `tag` (§6 table).")
        a("pub fn node_allows(kind: NodeKind, tag: u8) -> bool {")
        a("    match kind {")
        for k, props in spec.node_props.items():
            a(f"        NodeKind::{camel(k)} => matches!(tag, " + " | ".join(str(tags[p]) for p in props) + "),")
        a("    }")
        a("}")
        a("")
    # messages
    for n, m in spec.messages.items():
        L.extend(rs_doc(f"`{n}` · opcode {m['opcode']:#06x} · {m['dir']}" + (f"\n\n{m['doc']}" if m["doc"] else "")))
        a("#[derive(Clone, Debug, PartialEq, Eq, Default)]")
        if m["fields"]:
            a(f"pub struct {n} {{")
            for f in m["fields"]:
                a(f"    pub {f.name}: {rs_type(spec, f.t)},")
            a("}")
        else:
            a(f"pub struct {n};")
        a("")
        a(f"impl {n} {{")
        a("    pub fn enc(&self, w: &mut Writer) {")
        if not m["fields"]:
            a("        let _ = w;")
        for f in m["fields"]:
            for st in rs_enc(spec, f.t, f"&self.{f.name}"):
                a("        " + st)
        a("    }")
        a("    pub fn dec(r: &mut Reader) -> Result<Self, DecodeError> {")
        if not m["fields"]:
            a("        let _ = r;")
            a(f"        Ok({n})")
        else:
            for f in m["fields"]:
                a(f"        let {f.name} = {rs_dec(spec, f.t, f, f'{n}.{f.name}')};")
            a(f"        Ok({n} {{ " + ", ".join(f.name for f in m["fields"]) + " })")
        a("    }")
        a("    pub fn json(&self, o: &mut String) {")
        a(f'        o.push_str("{{\\"op\\":\\"{n}\\"");')
        for st in rs_fields_json(spec, m["fields"], lambda f: f"&self.{f.name}", True):
            a("        " + st)
        a("        o.push('}');")
        a("    }")
        a("}")
        a("")
        a(f"impl From<{n}> for Message {{")
        a(f"    fn from(m: {n}) -> Message {{ Message::{n}(m) }}")
        a("}")
        a("")
    a("/// Every message of the protocol.")
    a("#[derive(Clone, Debug, PartialEq, Eq)]")
    a("pub enum Message {")
    for n in spec.messages:
        a(f"    {n}({n}),")
    a("}")
    a("")
    a("impl Message {")
    a("    pub fn opcode(&self) -> u16 {")
    a("        match self {")
    for n, m in spec.messages.items():
        a(f"            Message::{n}(_) => {m['opcode']:#06x},")
    a("        }")
    a("    }")
    a("    pub fn name(&self) -> &'static str {")
    a("        match self {")
    for n in spec.messages:
        a(f'            Message::{n}(_) => "{n}",')
    a("        }")
    a("    }")
    a("    pub(crate) fn enc_body(&self, w: &mut Writer) {")
    a("        match self {")
    for n in spec.messages:
        a(f"            Message::{n}(m) => m.enc(w),")
    a("        }")
    a("    }")
    a("    pub(crate) fn dec_body(opcode: u16, r: &mut Reader) -> Result<Message, DecodeError> {")
    a("        Ok(match opcode {")
    for n, m in spec.messages.items():
        a(f"            {m['opcode']:#06x} => Message::{n}({n}::dec(r)?),")
    a("            _ => return Err(DecodeError::UnknownOpcode(opcode)),")
    a("        })")
    a("    }")
    a("    /// Whether `opcode` names a message of this protocol version.")
    a("    pub fn is_known(opcode: u16) -> bool {")
    a("        matches!(opcode, " + " | ".join(f"{m['opcode']:#06x}" for m in spec.messages.values()) + ")")
    a("    }")
    a("    pub(crate) fn json_into(&self, o: &mut String) {")
    a("        match self {")
    for n in spec.messages:
        a(f"            Message::{n}(m) => m.json(o),")
    a("        }")
    a("    }")
    a("}")
    return "\n".join(L) + "\n"


def is_tuple(v):
    return len(v["fields"]) == 1 and v["fields"][0].name == "value"


def rs_pattern(n, v):
    if is_tuple(v):
        return f"{n}::{v['name']}(value)"
    return f"{n}::{v['name']} {{ " + ", ".join(f.name for f in v["fields"]) + " }"


# ============================================================== Java codegen

def j_type(spec, t, boxed=False):
    k = spec.kind_of(t)
    if k == "prim":
        m = {"u8": "int", "u16": "int", "u32": "int", "i16": "int", "i32": "int", "u64": "long",
             "bool": "boolean", "str": "String", "bytes": "byte[]"}[t.name]
        if boxed:
            m = {"int": "Integer", "long": "Long", "boolean": "Boolean"}.get(m, m)
        return m
    if k == "flags":
        return "Integer" if boxed else "int"
    if k == "arr":
        return f"List<{j_type(spec, t.elem, True)}>"
    return f"UiProto.{t.name}"


class JGen:
    def __init__(self, spec):
        self.spec = spec
        self.n = 0

    def var(self, p):
        self.n += 1
        return f"{p}{self.n}"

    def enc(self, t, e):
        spec = self.spec
        k = spec.kind_of(t)
        if k == "prim":
            return [f"w.{t.name}({e});"] if t.name not in ("str", "bytes", "bool") else [f"w.{t.name}({e});"]
        if k == "enum":
            return [f"w.{spec.underlying(t)}({e}.code);"]
        if k == "flags":
            return [f"w.{spec.underlying(t)}({e});"]
        if k == "arr":
            x = self.var("x")
            inner = self.enc(t.elem, x)
            return [f"w.u32({e}.size());", f"for ({j_type(spec, t.elem, True)} {x} : {e}) {{"] + \
                ["    " + s for s in inner] + ["}"]
        return [f"{e}.enc(w);"]

    def checks(self, f, var, path):
        if f is None:
            return []
        out = []
        unsigned = f.t.kind == "prim" and f.t.name == "u32"
        conds = []
        for bound, op in ((f.lo, "<"), (f.hi, ">")):
            if bound is None:
                continue
            if unsigned:
                conds.append(f"Integer.compareUnsigned({var}, {bound}) {op} 0")
            else:
                conds.append(f"{var} {op} {bound}")
        if f.choices is not None:
            conds.append("!(" + " || ".join(f"{var} == {c}" for c in f.choices) + ")")
        if conds:
            out.append(f"if ({' || '.join(conds)}) throw UiProtoException.malformed(\"out of range: {path}\");")
        return out

    def dec(self, t, f, path):
        """(statements, expression)"""
        spec = self.spec
        k = spec.kind_of(t)
        if k == "prim":
            if t.name == "str":
                limit = f.max if f and f.max is not None else DEFAULT_STR_MAX
                return [], f'r.str({limit}, "{path}")'
            if t.name == "bytes":
                limit = f.max if f and f.max is not None else spec.max_message
                return [], f'r.bytes({limit}, "{path}")'
            if t.name == "bool":
                return [], f'r.bool("{path}")'
            v = self.var("v")
            st = [f"{j_type(spec, t)} {v} = r.{t.name}();"] + self.checks(f, v, path)
            return st, v
        if k == "enum":
            return [], f'UiProto.{t.name}.fromCode(r.{spec.underlying(t)}(), "{path}")'
        if k == "flags":
            v = self.var("v")
            mask = spec.flags[t.name]["mask"]
            return [f"int {v} = r.{spec.underlying(t)}();",
                    f'if (({v} & ~{mask:#x}) != 0) throw UiProtoException.malformed("bad flag bits: {path}");'], v
        if k == "arr":
            limit = f.max if f and f.max is not None else DEFAULT_ARR_MAX
            item = Field("item", t.elem, {"max": f.itemmax} if f and f.itemmax else {})
            n, v, i = self.var("n"), self.var("list"), self.var("i")
            ist, iexpr = self.dec(t.elem, item, path)
            st = [f'int {n} = r.count({limit}, {spec.min_size(t.elem)}, "{path}");',
                  f"List<{j_type(spec, t.elem, True)}> {v} = new ArrayList<>({n});",
                  f"for (int {i} = 0; {i} < {n}; {i}++) {{"] + ["    " + s for s in ist] + \
                 [f"    {v}.add({iexpr});", "}"]
            if f and f.sorted:
                ek = spec.kind_of(t.elem)
                if ek == "union":
                    cmp = f"Integer.compare({v}.get(k - 1).tag(), {v}.get(k).tag())"
                else:
                    first = spec.structs[t.elem.name]["fields"][0]
                    acc = lower_camel(first.name)
                    if first.t.name == "str":
                        cmp = f"UiReader.compareUtf8({v}.get(k - 1).{acc}(), {v}.get(k).{acc}())"
                    else:
                        cmp = f"Integer.compareUnsigned({v}.get(k - 1).{acc}(), {v}.get(k).{acc}())"
                st += [f"for (int k = 1; k < {v}.size(); k++) {{",
                       f'    if ({cmp} >= 0) throw UiProtoException.malformed("not sorted: {path}");', "}"]
            return st, f"List.copyOf({v})"
        return [], f"UiProto.{t.name}.dec(r)"

    def json(self, t, e):
        spec = self.spec
        k = spec.kind_of(t)
        if k == "prim":
            if t.name == "str":
                return [f"UiJson.str(o, {e});"]
            if t.name == "bytes":
                return [f"UiJson.hex(o, {e});"]
            if t.name == "bool":
                return [f'o.append({e} ? "true" : "false");']
            if t.name == "u32":
                return [f"o.append(Integer.toUnsignedString({e}));"]
            if t.name == "u64":
                return [f"o.append(Long.toUnsignedString({e}));"]
            return [f"o.append({e});"]
        if k == "enum":
            return [f"UiJson.str(o, {e}.wireName);"]
        if k == "flags":
            return [f"o.append(Integer.toUnsignedString({e}));"]
        if k == "arr":
            x, i = self.var("x"), self.var("i")
            inner = self.json(t.elem, x)
            return ["o.append('[');", f"for (int {i} = 0; {i} < {e}.size(); {i}++) {{",
                    f"    if ({i} > 0) o.append(',');", f"    {j_type(spec, t.elem, True)} {x} = {e}.get({i});"] + \
                ["    " + s for s in inner] + ["}", "o.append(']');"]
        return [f"{e}.json(o);"]

    def fields_json(self, fields, first_prefix):
        out = []
        for idx, f in enumerate(fields):
            sep = "" if idx == 0 and not first_prefix else ","
            out.append(f'o.append("{sep}\\"{f.name}\\":");')
            out += self.json(f.t, "this." + lower_camel(f.name))
        return out


def j_doc(doc, ind):
    if not doc:
        return []
    lines = doc.replace("*/", "* /").splitlines()
    if len(lines) == 1:
        return [f"{ind}/** {lines[0]} */"]
    return [f"{ind}/**"] + [f"{ind} * {x}" for x in lines] + [f"{ind} */"]


def gen_java(spec):
    g = JGen(spec)
    L = []
    a = L.append
    a("// @generated by scripts/gen-ui-proto.py from abi/ui-protocol.toml. Do not edit.")
    a("package com.example.evanscomputermod.computer.ui.proto;")
    a("")
    a("import java.util.ArrayList;")
    a("import java.util.List;")
    a("")
    a("/** UI protocol messages and their codec (docs/gui/UI_PROTOCOL.md, abi/ui-protocol.toml). */")
    a("@SuppressWarnings({\"unused\", \"PointlessBitwiseExpression\", \"ConstantConditions\"})")
    a("public final class UiProto {")
    a("    private UiProto() {}")
    a("")
    a(f"    public static final int VERSION = {spec.version};")
    a(f"    public static final int MAX_MESSAGE = {spec.max_message};")
    a("")
    a("    /** Opcode of every message. */")
    a("    public static final class Opcode {")
    a("        private Opcode() {}")
    for n, m in spec.messages.items():
        a(f"        public static final int {upper_snake(n)} = {m['opcode']:#06x};")
    a("    }")
    a("")
    for n, e in spec.enums.items():
        L.extend(j_doc(e["doc"], "    "))
        a(f"    public enum {n} {{")
        vals = e["values"]
        for i, (vn, code) in enumerate(vals):
            a(f'        {vn.upper()}({code}, "{vn}"){"," if i < len(vals) - 1 else ";"}')
        a("")
        a("        public final int code;")
        a("        public final String wireName;")
        a("")
        a(f"        {n}(int code, String wireName) {{ this.code = code; this.wireName = wireName; }}")
        a("")
        a(f"        public static {n} fromCode(int code, String path) throws UiProtoException {{")
        a(f"            for ({n} v : values()) if (v.code == code) return v;")
        a('            throw UiProtoException.malformed("bad enum value " + code + ": " + path);')
        a("        }")
        a("    }")
        a("")
    for n, f in spec.flags.items():
        a(f"    /** Bits of {n} ({f['type']}). */")
        a(f"    public static final class {n} {{")
        a(f"        private {n}() {{}}")
        for bn, b in f["bits"]:
            a(f"        public static final int {bn.upper()} = {b:#x};")
        a(f"        public static final int MASK = {f['mask']:#x};")
        a("    }")
        a("")

    def record(name, fields, implements, doc, extra, ind="    "):
        L.extend(j_doc(doc, ind))
        comps = ", ".join(f"{j_type(spec, f.t)} {lower_camel(f.name)}" for f in fields)
        a(f"{ind}public record {name}({comps}){implements} {{")
        for x in extra:
            a(ind + "    " + x if x else "")
        a(f"{ind}}}")
        a("")

    for n, s in spec.structs.items():
        body = ["public void enc(UiWriter w) {"]
        for f in s["fields"]:
            body += ["    " + x for x in g.enc(f.t, "this." + lower_camel(f.name))]
        body += ["}", "", f"public static {n} dec(UiReader r) throws UiProtoException {{"]
        for f in s["fields"]:
            st, ex = g.dec(f.t, f, f"{n}.{f.name}")
            body += ["    " + x for x in st] + [f"    {j_type(spec, f.t)} {lower_camel(f.name)}_ = {ex};"]
        body += [f"    return new {n}(" + ", ".join(lower_camel(f.name) + "_" for f in s["fields"]) + ");", "}", "",
                 "public void json(StringBuilder o) {", "    o.append('{');"]
        body += ["    " + x for x in g.fields_json(s["fields"], False)]
        body += ["    o.append('}');", "}"]
        record(n, s["fields"], "", s["doc"], body)

    for n, u in spec.unions.items():
        L.extend(j_doc(u["doc"], "    "))
        a(f"    public sealed interface {n} {{")
        a("        int tag();")
        a("        String name();")
        a("        void enc(UiWriter w);")
        a("        void json(StringBuilder o);")
        a("")
        a(f"        static {n} dec(UiReader r) throws UiProtoException {{")
        a(f"            int tag = r.{u['tag']}();")
        a("            switch (tag) {")
        for v in u["variants"]:
            a(f"                case {v['tag']}: {{")
            for f in v["fields"]:
                st, ex = g.dec(f.t, f, f"{n}.{v['name']}.{f.name}")
                for x in st:
                    a("                    " + x)
                a(f"                    {j_type(spec, f.t)} {lower_camel(f.name)}_ = {ex};")
            if v["validate"] == "node_props":
                a("                    for (UiProto.Prop p : props_) {")
                a("                        if (!UiProto.nodeAllows(kind_, p.tag()))")
                a(f'                            throw UiProtoException.malformed("prop not allowed: {n}.{v["name"]}.props");')
                a("                    }")
            a(f"                    return new {v['name']}(" + ", ".join(lower_camel(f.name) + "_" for f in v["fields"]) + ");")
            a("                }")
        a(f'                default: throw UiProtoException.malformed("bad enum value " + tag + ": {n}");')
        a("            }")
        a("        }")
        a("")
        for v in u["variants"]:
            body = [f"@Override public int tag() {{ return {v['tag']}; }}",
                    f'@Override public String name() {{ return "{v["name"]}"; }}',
                    "@Override public void enc(UiWriter w) {", f"    w.{u['tag']}({v['tag']});"]
            for f in v["fields"]:
                body += ["    " + x for x in g.enc(f.t, "this." + lower_camel(f.name))]
            body += ["}", "@Override public void json(StringBuilder o) {",
                     f'    o.append("{{\\"type\\":\\"{v["name"]}\\"");']
            body += ["    " + x for x in g.fields_json(v["fields"], True)]
            body += ["    o.append('}');", "}"]
            record(v["name"], v["fields"], f" implements {n}", v["doc"], body, "        ")
        a("    }")
        a("")
    if spec.node_props:
        tags = {v["name"]: v["tag"] for v in spec.unions["Prop"]["variants"]}
        a("    /** Whether a node of {@code kind} accepts the prop with tag {@code tag} (§6 table). */")
        a("    public static boolean nodeAllows(NodeKind kind, int tag) {")
        a("        return switch (kind) {")
        for k, props in spec.node_props.items():
            a(f"            case {k.upper()} -> " + " || ".join(f"tag == {tags[p]}" for p in props) + ";")
        a("        };")
        a("    }")
        a("")
    a("    /** A protocol message. */")
    a("    public sealed interface Message permits " + ", ".join(spec.messages) + " {")
    a("        int opcode();")
    a("        String name();")
    a("        void encBody(UiWriter w);")
    a("        void json(StringBuilder o);")
    a("    }")
    a("")
    for n, m in spec.messages.items():
        body = [f"@Override public int opcode() {{ return {m['opcode']:#06x}; }}",
                f'@Override public String name() {{ return "{n}"; }}',
                "@Override public void encBody(UiWriter w) {"]
        for f in m["fields"]:
            body += ["    " + x for x in g.enc(f.t, "this." + lower_camel(f.name))]
        body += ["}", f"static {n} dec(UiReader r) throws UiProtoException {{"]
        for f in m["fields"]:
            st, ex = g.dec(f.t, f, f"{n}.{f.name}")
            body += ["    " + x for x in st] + [f"    {j_type(spec, f.t)} {lower_camel(f.name)}_ = {ex};"]
        body += [f"    return new {n}(" + ", ".join(lower_camel(f.name) + "_" for f in m["fields"]) + ");", "}",
                 "@Override public void json(StringBuilder o) {", f'    o.append("{{\\"op\\":\\"{n}\\"");']
        body += ["    " + x for x in g.fields_json(m["fields"], True)]
        body += ["    o.append('}');", "}"]
        doc = f"{n} · opcode {m['opcode']:#06x} · {m['dir']}" + (f". {m['doc']}" if m["doc"] else "")
        record(n, m["fields"], " implements Message", doc, body)
    a("    /** Whether {@code opcode} names a message of this protocol version. */")
    a("    public static boolean isKnown(int opcode) {")
    a("        return switch (opcode) {")
    a("            case " + ", ".join(f"{m['opcode']:#06x}" for m in spec.messages.values()) + " -> true;")
    a("            default -> false;")
    a("        };")
    a("    }")
    a("")
    a("    static Message decBody(int opcode, UiReader r) throws UiProtoException {")
    a("        return switch (opcode) {")
    for n, m in spec.messages.items():
        a(f"            case {m['opcode']:#06x} -> {n}.dec(r);")
    a("            default -> throw UiProtoException.unknownOpcode(opcode);")
    a("        };")
    a("    }")
    a("}")
    return "\n".join(L) + "\n"


# ============================================================== vectors

class Sampler:
    """Deterministic sample values that satisfy the contract."""

    STR_SAMPLES = ['née "q" \\ ✓', "tab\tnl\n\U0001F600", "plain"]

    def __init__(self, spec):
        self.spec = spec
        self.seed = 0

    def next(self):
        self.seed += 1
        return self.seed

    def value(self, t, f, full):
        spec = self.spec
        k = spec.kind_of(t)
        s = self.next()
        if k == "prim":
            n = t.name
            if n == "bool":
                return full
            if n in ("str",):
                if not full:
                    return ""
                txt = f"{f.name if f else 's'} " + self.STR_SAMPLES[s % len(self.STR_SAMPLES)]
                limit = f.max if f and f.max is not None else DEFAULT_STR_MAX
                while len(txt.encode()) > limit:
                    txt = txt[:-1]
                return txt
            if n == "bytes":
                return bytes([0, 1, 0x7F, 0x80, 0xFF]) if full else b""
            lo, hi = INT_BOUNDS[n]
            if f is not None and f.choices is not None:
                return f.choices[-1] if full else f.choices[0]
            flo = f.lo if f is not None and f.lo is not None else lo
            fhi = f.hi if f is not None and f.hi is not None else hi
            if not full:
                return max(flo, 0) if flo <= 0 <= fhi else flo
            if (flo, fhi) != (lo, hi) and fhi - flo < 1000:
                return fhi
            candidates = {"u8": 0x80 + s % 0x70, "u16": 0x8000 + s, "u32": 0x80000000 + s * 7,
                          "u64": 0x8000000000000000 + s, "i16": -(s + 3), "i32": -(s * 1000 + 7)}
            v = candidates[n]
            if not flo <= v <= fhi:
                v = min(fhi, flo + 1000 + s)
            return v
        if k == "enum":
            vals = spec.enums[t.name]["values"]
            return vals[-1][0] if full else vals[0][0]
        if k == "flags":
            return spec.flags[t.name]["mask"] if full else 0
        if k == "arr":
            if not full:
                return []
            if spec.kind_of(t.elem) == "struct" and f and f.sorted:
                first = spec.structs[t.elem.name]["fields"][0]
                out = []
                for key in ("accent", "bg"):
                    item = self.value(t.elem, None, True)
                    item[first.name] = key
                    out.append(item)
                return out
            item = Field("item", t.elem, {"max": f.itemmax} if f and f.itemmax else {})
            limit = f.max if f and f.max is not None else 2
            return [self.value(t.elem, item, True) for _ in range(min(2, limit))]
        if k == "struct":
            return {g.name: self.value(g.t, g, full) for g in spec.structs[t.name]["fields"]}
        if k == "union":
            raise AssertionError("unions are sampled by hand")
        raise AssertionError(t)


def all_props(spec, kind):
    """One prop of every kind allowed on `kind`, sorted, with distinctive values."""
    sampler = Sampler(spec)
    out = []
    allowed = spec.node_props[kind]
    for v in spec.unions["Prop"]["variants"]:
        if v["name"] not in allowed:
            continue
        f = v["fields"][0]
        if v["name"] == "Points":
            val = [{"x": -5, "y": 7}, {"x": 300, "y": -2}, {"x": 0, "y": 0}]
        elif v["name"] == "Src":
            val = {"x": 1, "y": 2, "w": 16, "h": 8}
        elif v["name"] == "Text":
            val = "Add été ✓"
        else:
            val = sampler.value(f.t, f, True)
        out.append({"type": v["name"], "value": val})
    return out


def valid_vectors(spec):
    vecs = []
    for n, m in spec.messages.items():
        for full in (True, False):
            if n == "TreeOps":
                continue
            s = Sampler(spec)
            val = {f.name: s.value(f.t, f, full) for f in m["fields"]}
            vecs.append((f"{n}{'' if full else '.empty'}", n, val))
    # TreeOps: every node kind with every prop it accepts, then Set / Move / Remove.
    ops = []
    for i, (kind, _) in enumerate(spec.enums["NodeKind"]["values"]):
        ops.append({"type": "Create", "id": i + 1, "parent": 0, "index": 0xFFFFFFFF, "kind": kind,
                    "props": all_props(spec, kind)})
    vecs.append(("TreeOps.create_every_kind", "TreeOps", {"surface": 1, "ops": ops}))
    vecs.append(("TreeOps.edit", "TreeOps", {"surface": 0x80000001, "ops": [
        {"type": "Set", "id": 3, "props": [{"type": "Text", "value": "Saved"}, {"type": "Color", "value": 0xFF8800FF}]},
        {"type": "Set", "id": 2, "props": []},
        {"type": "Move", "id": 3, "parent": 1, "index": 0},
        {"type": "Remove", "id": 0xFFFFFFFF},
    ]}))
    vecs.append(("TreeOps.empty", "TreeOps", {"surface": 7, "ops": []}))
    return vecs


def header(op, n, flags=0):
    return op.to_bytes(2, "little") + flags.to_bytes(2, "little") + n.to_bytes(4, "little")


def u32(v):
    return v.to_bytes(4, "little")


def s32(b):
    return u32(len(b)) + b


def invalid_vectors(spec):
    op = {n: m["opcode"] for n, m in spec.messages.items()}

    def msg(name, body):
        return header(op[name], len(body)) + body

    def prop(tag, raw):
        return bytes([tag]) + raw

    v = []
    v.append(("unknown_opcode", header(0x7777, 0)))
    v.append(("nonzero_flags", header(op["Commit"], 4, flags=1) + u32(1)))
    v.append(("too_large", header(op["SaveState"], spec.max_message + 1)))
    v.append(("trailing_byte", msg("Commit", u32(1) + b"\x00")))
    v.append(("short_body", msg("Commit", b"\x01\x00\x00")))
    v.append(("truncated_buffer", header(op["Commit"], 4) + b"\x01\x00"))
    v.append(("incomplete_header", b"\x07\x01\x04"))
    v.append(("bad_bool", msg("Focus", u32(1) + b"\x02")))
    v.append(("bad_enum", msg("Hello", b"\x01\x00" + b"\x07" + u32(0))))
    v.append(("bad_enum_u16", msg("Error", b"\x00\x00" + s32(b""))))
    v.append(("bad_flag_bit", msg("Configure", u32(1) + u32(2) + u32(10) + u32(10) + u32(0x10))))
    v.append(("bad_utf8", msg("SurfaceSetTitle", u32(1) + s32(b"\xff\xfe"))))
    v.append(("surrogate_utf8", msg("SurfaceSetTitle", u32(1) + s32(b"\xed\xa0\x80"))))
    v.append(("overlong_utf8", msg("SurfaceSetTitle", u32(1) + s32(b"\xc0\xaf"))))
    v.append(("str_too_long", msg("HotkeyClaim", b"\x01" + s32(b"x" * 65))))
    v.append(("str_len_past_body", msg("HotkeyClaim", b"\x01" + u32(50) + b"abc")))
    v.append(("range_high", msg("Hotkey", b"\x07\x01")))
    v.append(("range_low", msg("Hotkey", b"\x00\x01")))
    v.append(("range_zero_surface", msg("SurfaceCreate", u32(0) + bytes(4) + u32(0) + bytes(16) + b"\xff\x01" + s32(b""))))
    v.append(("negative_size", msg("Configure", u32(1) + u32(1) + (-1 & 0xFFFFFFFF).to_bytes(4, "little") + u32(1) + u32(0))))
    v.append(("array_too_long", msg("BindChord", u32(1) + b"\x00\x00" + u32(5) + bytes(10))))
    v.append(("count_exceeds_body", msg("Stack", u32(1000000))))
    v.append(("itemmax_exceeded", msg("Notify", s32(b"") * 4 + u32(1) + s32(b"a" * 65))))
    v.append(("unsorted_theme", msg("SetTheme", s32(b"t") + u32(2) + s32(b"b") + u32(1) + s32(b"a") + u32(2))))
    v.append(("duplicate_theme_key", msg("SetTheme", s32(b"t") + u32(2) + s32(b"a") + u32(1) + s32(b"a") + u32(2))))
    v.append(("unknown_union_tag", msg("TreeOps", u32(1) + u32(1) + b"\x09")))
    v.append(("unknown_prop_tag", msg("TreeOps", u32(1) + u32(1) + b"\x01" + u32(1) + u32(1) + prop(23, u32(0)))))
    v.append(("prop_tag_zero", msg("TreeOps", u32(1) + u32(1) + b"\x01" + u32(1) + u32(1) + prop(0, u32(0)))))
    v.append(("unsorted_props", msg("TreeOps", u32(1) + u32(1) + b"\x01" + u32(1) + u32(2) +
                                    prop(2, u32(5)) + prop(1, u32(5)))))
    v.append(("duplicate_props", msg("TreeOps", u32(1) + u32(1) + b"\x01" + u32(1) + u32(2) +
                                     prop(1, u32(5)) + prop(1, u32(6)))))
    v.append(("prop_not_allowed", msg("TreeOps", u32(1) + u32(1) + b"\x00" + u32(1) + u32(0) + u32(0) + b"\x00" +
                                      u32(1) + prop(13, s32(b"hi")))))
    v.append(("size_not_supported", msg("TreeOps", u32(1) + u32(1) + b"\x00" + u32(1) + u32(0) + u32(0) + b"\x02" +
                                        u32(1) + prop(15, b"\x0a"))))
    v.append(("node_id_zero", msg("TreeOps", u32(1) + u32(1) + b"\x03" + u32(0))))
    v.append(("too_many_points", msg("TreeOps", u32(1) + u32(1) + b"\x01" + u32(1) + u32(1) +
                                     prop(21, u32(257) + bytes(257 * 8)))))
    v.append(("bad_scale", msg("TreeOps", u32(1) + u32(1) + b"\x01" + u32(1) + u32(1) + prop(7, b"\x00"))))
    return v


def gen_vectors(spec):
    valid_lines = [
        "# UI protocol test vectors: generated by scripts/gen-ui-proto.py from abi/ui-protocol.toml.",
        "# One per line: <name> <hex of one whole message> <canonical JSON of its fields>.",
        "# Canonical JSON: fields in contract order, \"op\" first; unions {\"type\":..} first; enums by name;",
        "# flags and integers in decimal (unsigned for u*); bytes as lowercase hex; strings escape only",
        "# \\\" \\\\ and U+0000..U+001F (as \\u00xx). Codecs must decode each to that JSON and re-encode",
        "# to the same bytes.",
    ]
    covered = set()
    for name, msgname, val in valid_vectors(spec):
        enc = py_encode_message(spec, msgname, val)
        dn, dv = py_decode(spec, enc)
        assert dn == msgname and dv == val, (name, dv, val)
        valid_lines.append(f"{name} {enc.hex()} {py_json_message(spec, msgname, val)}")
        covered.add(msgname)
    assert covered == set(spec.messages), set(spec.messages) - covered
    invalid_lines = [
        "# UI protocol vectors that every decoder must reject: generated by scripts/gen-ui-proto.py.",
        "# One per line: <name> <hex>. Decoding the hex as one whole message must fail",
        "# (a buffer that is merely incomplete counts as failing).",
    ]
    for name, raw in invalid_vectors(spec):
        try:
            py_decode(spec, raw)
        except Bad:
            pass
        else:
            raise AssertionError(f"invalid vector {name} decodes")
        invalid_lines.append(f"{name} {raw.hex()}")
    return "\n".join(valid_lines) + "\n", "\n".join(invalid_lines) + "\n"


def main():
    check = "--check" in sys.argv
    spec = Spec(tomllib.loads(SPEC.read_text(encoding="utf-8")))
    valid, invalid = gen_vectors(spec)
    outputs = {
        RUST_OUT: gen_rust(spec),
        JAVA_OUT: gen_java(spec),
        VEC_DIR / "valid.txt": valid,
        VEC_DIR / "invalid.txt": invalid,
    }
    stale = []
    for path, text in outputs.items():
        old = path.read_text(encoding="utf-8").replace(chr(13) + chr(10), chr(10)) if path.exists() else None
        if old != text:
            stale.append(path)
            if not check:
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(text, encoding="utf-8", newline="\n")
    for p in stale:
        print(("stale: " if check else "wrote ") + str(p.relative_to(ROOT)))
    if check and stale:
        sys.exit(1)


if __name__ == "__main__":
    main()
