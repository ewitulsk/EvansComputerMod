#!/usr/bin/env python3
"""Generate the ECM font tables from the glyph sources.

Usage:
  scripts/gen-ui-fonts.py                 write the Rust and Java tables
  scripts/gen-ui-fonts.py --check         exit 1 if the tables are stale
  scripts/gen-ui-fonts.py --extract-mono  (re)write mono-8x16.txt from the terminal font PNG
  scripts/gen-ui-fonts.py --preview TEXT  print TEXT in every design (ASCII art)

Sources live in src/main/resources/assets/evanscomputermod/ecm_fonts/ (see
fonts.toml and the format notes at the top of ui-regular.txt). Outputs:
  rust/crates/ecm-ui-proto/src/fonts_data.rs
  src/main/java/com/example/evanscomputermod/computer/ui/font/UiFontData.java

Glyph rows are stored as u16 bitmasks, bit 0 = leftmost column, `height` rows
per glyph. The metrics hash (FNV-1a 32 over a canonical serialisation, below)
is recomputed by the Rust and Java tests from their own tables.
"""
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SRC = ROOT / "src/main/resources/assets/evanscomputermod/ecm_fonts"
RUST_OUT = ROOT / "rust/crates/ecm-ui-proto/src/fonts_data.rs"
JAVA_OUT = ROOT / "src/main/java/com/example/evanscomputermod/computer/ui/font/UiFontData.java"
TERMINAL_PNG = ROOT / "src/main/resources/assets/evanscomputermod/textures/terminal_font.png"
FONT_ORDER = ["ui", "mono"]  # index = protocol Font enum value


def parse_char(tok):
    if tok.startswith("U+") and len(tok) > 2:
        return int(tok[2:], 16)
    assert len(tok) == 1, f"bad character {tok!r}"
    return ord(tok)


class Design:
    def __init__(self, path):
        self.path = path
        self.glyphs = {}  # cp -> (width, [row strings, height long])
        self.marks = {}
        self.missing = None
        self.height = self.ascent = None
        self.spacing = 0
        self.name = path.stem
        self.parse(path.read_text(encoding="utf-8").splitlines())

    def parse(self, lines):
        i = 0
        compose = []
        while i < len(lines):
            line = lines[i].strip()
            i += 1
            if not line or (line.startswith("#") and not set(line) <= {"#", "."}):
                continue
            parts = line.split()
            kw = parts[0]
            if kw == "font":
                self.name = parts[1]
            elif kw == "height":
                self.height = int(parts[1])
            elif kw == "ascent":
                self.ascent = int(parts[1])
            elif kw == "spacing":
                self.spacing = int(parts[1])
            elif kw in ("glyph", "missing", "mark"):
                if kw == "glyph":
                    cp, spec = parse_char(parts[1]), parts[2]
                elif kw == "missing":
                    cp, spec = None, parts[1]
                else:
                    cp, spec = parts[1], "@0"
                rows = []
                while i < len(lines) and lines[i].strip():
                    r = lines[i].strip()
                    assert set(r) <= {"#", "."}, f"{self.path.name}:{i + 1}: bad row {r!r}"
                    rows.append(r)
                    i += 1
                if spec.startswith("w"):
                    assert not rows, f"{self.path.name}:{i}: w-glyph with rows"
                    width, top = int(spec[1:]), 0
                else:
                    assert spec.startswith("@"), spec
                    top = int(spec[1:])
                    assert rows, f"{self.path.name}:{i}: glyph without rows"
                    width = len(rows[0])
                    for r in rows:
                        assert len(r) == width, f"{self.path.name}:{i}: ragged glyph {parts[1]}"
                if kw == "mark":
                    self.marks[cp] = rows
                    continue
                assert top + len(rows) <= self.height, f"{self.path.name}:{i}: glyph {parts[1]} too tall"
                full = ["." * width] * top + rows + ["." * width] * (self.height - top - len(rows))
                if kw == "missing":
                    self.missing = (width, full)
                else:
                    assert cp not in self.glyphs, f"{self.path.name}: duplicate glyph {parts[1]}"
                    self.glyphs[cp] = (width, full)
            elif kw == "alias":
                compose.append(("alias", parse_char(parts[1]), parse_char(parts[2])))
            elif kw == "compose":
                compose.append(("compose", parse_char(parts[1]), parse_char(parts[2]), parts[3], int(parts[4][1:])))
            else:
                raise ValueError(f"{self.path.name}:{i}: unknown line {line!r}")
        for c in compose:
            if c[0] == "alias":
                self.glyphs[c[1]] = self.glyphs[c[2]]
            else:
                _, cp, base, mark, row = c
                self.glyphs[cp] = self.overlay(self.glyphs[base], self.marks[mark], row)
        assert self.missing, f"{self.path.name}: no missing glyph"

    def overlay(self, base, mark, row):
        bw, brows = base
        mw = len(mark[0])
        x = (bw - mw) // 2
        left = min(0, x)
        width = max(bw, x + mw) - left
        out = [list("." * width) for _ in range(self.height)]
        for y, r in enumerate(brows):
            for cx, ch in enumerate(r):
                if ch == "#":
                    out[y][cx - left] = "#"
        for y, r in enumerate(mark):
            for cx, ch in enumerate(r):
                if ch == "#":
                    out[row + y][x + cx - left] = "#"
        return width, ["".join(r) for r in out]


def bits(row):
    v = 0
    for i, ch in enumerate(row):
        if ch == "#":
            v |= 1 << i
    assert v < 1 << 16, "glyph wider than 16"
    return v


def load():
    cfg = tomllib.loads((SRC / "fonts.toml").read_text(encoding="utf-8"))
    designs = []   # (font, base size, Design)
    sizes = []     # (font index, size, design index, scale)
    for fi, fname in enumerate(FONT_ORDER):
        f = cfg["fonts"][fname]
        first = len(designs)
        for d in f["designs"]:
            design = Design(SRC / d["file"])
            assert design.height == d["size"], f"{d['file']}: height {design.height} != size {d['size']}"
            designs.append((fi, d["size"], design))
        for s in f["sizes"]:
            best = None
            for di in range(first, len(designs)):
                base = designs[di][1]
                if s % base == 0 and (best is None or base > designs[best][1]):
                    best = di
            assert best is not None, f"{fname} size {s} is no multiple of a design"
            sizes.append((fi, s, best, s // designs[best][1]))
    return cfg, designs, sizes


def metrics_hash(designs, sizes):
    h = 0x811C9DC5

    def feed(*vals):
        nonlocal h
        for v in vals:
            for b in v.to_bytes(4, "little"):
                h = ((h ^ b) * 0x01000193) & 0xFFFFFFFF

    for fi, base, d in designs:
        feed(fi, base, d.height, d.ascent, d.spacing, d.missing[0], *[bits(r) for r in d.missing[1]])
        for cp in sorted(d.glyphs):
            w, rows = d.glyphs[cp]
            feed(cp, w, *[bits(r) for r in rows])
    for s in sizes:
        feed(*s)
    return h


def gen_rust(cfg, designs, sizes, h):
    L = ["// @generated by scripts/gen-ui-fonts.py from src/main/resources/assets/evanscomputermod/ecm_fonts. Do not edit.",
         "",
         "use super::Design;",
         "",
         f"pub const METRICS_VERSION: u32 = {cfg['metrics_version']};",
         f"pub const METRICS_HASH: u32 = {h:#010x};",
         ""]
    names = []
    for di, (fi, base, d) in enumerate(designs):
        cps = sorted(d.glyphs)
        rows = []
        table = []
        for cp in cps:
            w, rs = d.glyphs[cp]
            table.append(f"({cp:#x}, {w}, {len(rows)})")
            rows += [bits(r) for r in rs]
        missing_off = len(rows)
        rows += [bits(r) for r in d.missing[1]]
        name = d.name.upper().replace("-", "_")
        names.append(name)
        L.append(f"static {name}_GLYPHS: &[(u32, u8, u16)] = &[")
        for k in range(0, len(table), 8):
            L.append("    " + ", ".join(table[k:k + 8]) + ",")
        L.append("];")
        L.append(f"static {name}_ROWS: &[u16] = &[")
        for k in range(0, len(rows), 16):
            L.append("    " + ", ".join(f"{r:#x}" for r in rows[k:k + 16]) + ",")
        L.append("];")
        L.append(f"pub static {name}: Design = Design {{ name: \"{d.name}\", font: {fi}, base_size: {base}, "
                 f"height: {d.height}, ascent: {d.ascent}, spacing: {d.spacing}, "
                 f"glyphs: {name}_GLYPHS, rows: {name}_ROWS, missing_width: {d.missing[0]}, missing_offset: {missing_off} }};")
        L.append("")
    L.append("/// Every design, in the order the metrics hash covers them.")
    L.append("pub static DESIGNS: &[&Design] = &[" + ", ".join(f"&{n}" for n in names) + "];")
    L.append("")
    L.append("/// (font, size, design index, scale) for every supported size.")
    L.append("pub static SIZES: &[(u8, u8, u8, u8)] = &[" + ", ".join(f"({a}, {b}, {c}, {e})" for a, b, c, e in sizes) + "];")
    return "\n".join(L) + "\n"


def jstr(vals):
    out = []
    for v in vals:
        out.append("\\u%04x" % v)
    s = "".join(out)
    # split long literals over lines
    chunks = [s[i:i + 96] for i in range(0, len(s), 96)]
    return "\n            + ".join(f'"{c}"' for c in chunks) if chunks else '""'


def gen_java(cfg, designs, sizes, h):
    L = ["// @generated by scripts/gen-ui-fonts.py from src/main/resources/assets/evanscomputermod/ecm_fonts. Do not edit.",
         "package com.example.evanscomputermod.computer.ui.font;",
         "",
         "/** Glyph tables of the ECM fonts (see {@link UiFonts}). Strings hold u16 values, one per char. */",
         "final class UiFontData {",
         "    private UiFontData() {}",
         "",
         f"    static final int METRICS_VERSION = {cfg['metrics_version']};",
         f"    static final int METRICS_HASH = {h:#010x};",
         ""]
    entries = []
    for di, (fi, base, d) in enumerate(designs):
        cps = sorted(d.glyphs)
        rows = []
        glyph_vals = []
        for cp in cps:
            w, rs = d.glyphs[cp]
            assert cp < 0x10000
            glyph_vals += [cp, w, len(rows)]
            rows += [bits(r) for r in rs]
        missing_off = len(rows)
        rows += [bits(r) for r in d.missing[1]]
        name = d.name.upper().replace("-", "_")
        L.append(f"    /** {d.name}: (codepoint, width, row offset) per glyph, sorted. */")
        L.append(f"    static final String {name}_GLYPHS = {jstr(glyph_vals)};")
        L.append(f"    static final String {name}_ROWS = {jstr(rows)};")
        L.append("")
        entries.append(f'new UiFonts.Design("{d.name}", {fi}, {base}, {d.height}, {d.ascent}, {d.spacing}, '
                       f'{name}_GLYPHS, {name}_ROWS, {d.missing[0]}, {missing_off})')
    L.append("    static UiFonts.Design[] designs() {")
    L.append("        return new UiFonts.Design[] {")
    for e in entries:
        L.append(f"            {e},")
    L.append("        };")
    L.append("    }")
    L.append("")
    L.append("    /** {font, size, design index, scale} per supported size. */")
    L.append("    static final int[][] SIZES = {" + ", ".join("{%d, %d, %d, %d}" % s for s in sizes) + "};")
    L.append("}")
    return "\n".join(L) + "\n"


def extract_mono():
    from PIL import Image
    im = Image.open(TERMINAL_PNG).convert("RGBA")
    px = im.load()
    out = ["# ECM \"mono\" font: the 8x16 terminal font (evanscomputermod:terminal,",
           "# textures/terminal_font.png), extracted by scripts/gen-ui-fonts.py --extract-mono.",
           "# Format: see ui-regular.txt. Every glyph is 8 wide (advance 8).",
           "",
           "font mono",
           "height 16",
           "ascent 14",
           "spacing 0",
           "",
           "missing @2",
           "#######.", "#.....#.", "#.....#.", "#.....#.", "#.....#.", "#.....#.", "#.....#.", "#.....#.",
           "#.....#.", "#.....#.", "#.....#.", "#######.",
           ""]
    for cp in list(range(0x20, 0x7F)) + list(range(0xA0, 0x100)):
        cx, cy = (cp % 16) * 8, (cp // 16) * 16
        rows = ["".join("#" if px[cx + x, cy + y][3] > 127 else "." for x in range(8)) for y in range(16)]
        ink = [i for i, r in enumerate(rows) if "#" in r]
        tok = f"U+{cp:04X}" if cp in (0x20, 0x23, 0xA0, 0xAD) or cp > 0x7E and cp < 0xA1 else chr(cp)
        if not ink:
            out += [f"glyph {tok} w8", ""]
            continue
        out.append(f"glyph {tok} @{ink[0]}")
        out += rows[ink[0]:ink[-1] + 1]
        out.append("")
    (SRC / "mono-8x16.txt").write_text("\n".join(out), encoding="utf-8", newline="\n")
    print("wrote", (SRC / "mono-8x16.txt").relative_to(ROOT))


def preview(text):
    _, designs, _ = load()
    for _, _, d in designs:
        print(f"-- {d.name}")
        lines = [""] * d.height
        for ch in text:
            w, rows = d.glyphs.get(ord(ch), d.missing)
            for y in range(d.height):
                lines[y] += rows[y].replace(".", " ") + " " * d.spacing
        print("\n".join(lines))


def main():
    if "--extract-mono" in sys.argv:
        extract_mono()
        return
    if "--preview" in sys.argv:
        preview(sys.argv[sys.argv.index("--preview") + 1])
        return
    cfg, designs, sizes = load()
    h = metrics_hash(designs, sizes)
    if h != cfg["metrics_hash"]:
        print(f"metrics changed: hash is {h:#010x}, fonts.toml says {cfg['metrics_hash']:#010x}.\n"
              f"Bump metrics_version and set metrics_hash = {h:#010x} in fonts.toml.")
        sys.exit(1)
    outputs = {RUST_OUT: gen_rust(cfg, designs, sizes, h), JAVA_OUT: gen_java(cfg, designs, sizes, h)}
    check = "--check" in sys.argv
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
