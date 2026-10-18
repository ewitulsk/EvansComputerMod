#!/usr/bin/env python3
"""Compose the Tech Village Radio tracks (docs/MUSIC_CREDITS.md) and encode them as tagged MP3s.

Usage: python scripts/make-village-music.py [OUT_DIR]   (default src/main/resources/evanscomputermod/village/music)

Everything here is made from code in this file: the notes, the synthesis, the cover art.
No samples or recordings from anywhere else are used, so the tracks are original works,
dedicated to the public domain (CC0 1.0) like the rest of the mod's generated assets.
Needs ffmpeg with libmp3lame on PATH. Deterministic: the same script makes the same audio.
"""
import math
import os
import random
import struct
import subprocess
import sys
import tempfile
import zlib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OUT = Path(sys.argv[1]) if len(sys.argv) > 1 else ROOT / "src/main/resources/evanscomputermod/village/music"
RATE = 32000
ARTIST = "Tech Village Band"
ALBUM = "Tech Village Radio"
YEAR = "2026"

NOTE = {"C": 0, "D": 2, "E": 4, "F": 5, "G": 7, "A": 9, "B": 11}


def freq(name):
    """'A4', 'C#5', 'Bb3' -> Hz."""
    n = NOTE[name[0]]
    rest = name[1:]
    if rest.startswith("#"):
        n += 1
        rest = rest[1:]
    elif rest.startswith("b"):
        n -= 1
        rest = rest[1:]
    octave = int(rest)
    return 440.0 * 2 ** ((n + 12 * (octave + 1) - 69) / 12)


def env(t, dur, a=0.01, r=0.08):
    if t < a:
        return t / a
    if t > dur - r:
        return max(0.0, (dur - t) / r)
    return 1.0


def osc(kind, phase):
    p = phase % 1.0
    if kind == "sine":
        return math.sin(2 * math.pi * p)
    if kind == "square":
        return 1.0 if p < 0.5 else -1.0
    if kind == "pulse":
        return 1.0 if p < 0.25 else -1.0
    if kind == "saw":
        return 2 * p - 1
    if kind == "tri":
        return 4 * abs(p - 0.5) - 1
    raise ValueError(kind)


class Mix:
    def __init__(self, seconds):
        self.buf = [0.0] * int(seconds * RATE)

    def note(self, start, dur, f, kind="sine", vol=0.2, a=0.01, r=0.08, vibrato=0.0):
        i0 = int(start * RATE)
        n = int(dur * RATE)
        phase = 0.0
        for k in range(n):
            i = i0 + k
            if i >= len(self.buf):
                break
            t = k / RATE
            fm = f * (1 + vibrato * math.sin(2 * math.pi * 5.5 * t))
            phase += fm / RATE
            self.buf[i] += osc(kind, phase) * vol * env(t, dur, a, r)

    def drum(self, start, kind, vol=0.3):
        rnd = random.Random(int(start * 1000) ^ hash(kind) & 0xFFFF)
        i0 = int(start * RATE)
        if kind == "kick":
            n = int(0.18 * RATE)
            phase = 0.0
            for k in range(n):
                t = k / RATE
                f = 45 + 110 * math.exp(-t * 30)
                phase += f / RATE
                if i0 + k < len(self.buf):
                    self.buf[i0 + k] += math.sin(2 * math.pi * phase) * vol * math.exp(-t * 14)
        elif kind == "snare":
            n = int(0.15 * RATE)
            for k in range(n):
                t = k / RATE
                if i0 + k < len(self.buf):
                    self.buf[i0 + k] += (rnd.uniform(-1, 1) * 0.8 + math.sin(2 * math.pi * 190 * t) * 0.3) * vol * math.exp(-t * 22)
        elif kind == "hat":
            n = int(0.05 * RATE)
            last = 0.0
            for k in range(n):
                t = k / RATE
                x = rnd.uniform(-1, 1)
                hp = x - last
                last = x
                if i0 + k < len(self.buf):
                    self.buf[i0 + k] += hp * vol * 0.5 * math.exp(-t * 70)

    def echo(self, delay, feedback):
        d = int(delay * RATE)
        for i in range(d, len(self.buf)):
            self.buf[i] += self.buf[i - d] * feedback

    def lowpass(self, hz):
        """Two one-pole low-passes: soften the naive oscillators' aliased top end."""
        a = 1 - math.exp(-2 * math.pi * hz / RATE)
        for _ in range(2):
            y = 0.0
            for i, x in enumerate(self.buf):
                y += a * (x - y)
                self.buf[i] = y

    def pcm(self):
        self.lowpass(5500)
        peak = max(1e-9, max(abs(x) for x in self.buf))
        g = 0.89 / peak
        out = bytearray()
        for x in self.buf:
            y = math.tanh(x * g * 1.1) / math.tanh(1.1)
            out += struct.pack("<h", int(max(-1, min(1, y)) * 32767))
        return bytes(out)


def chord_notes(root, minor=False):
    third = 3 if minor else 4
    return [root, root * 2 ** (third / 12), root * 2 ** (7 / 12)]


def redstone_sunrise():
    """Chiptune: square arpeggios over a pulse bass, C major, 112 BPM."""
    bpm, bars = 112, 20
    beat = 60 / bpm
    m = Mix(bars * 4 * beat + 1.5)
    prog = [("C3", False), ("A2", True), ("F2", False), ("G2", False)]
    melody = ["E5", "G5", "C6", "B5", "A5", "G5", "E5", "D5", "C5", "E5", "G5", "A5", "G5", "F5", "E5", "D5"]
    for bar in range(bars):
        root, minor = prog[bar % 4]
        f0 = freq(root)
        t0 = bar * 4 * beat
        for k in range(16):
            n = chord_notes(f0 * 4, minor)[k % 3] * (2 if (k // 3) % 2 else 1)
            m.note(t0 + k * beat / 4, beat / 4 * 0.9, n, "square", 0.05, r=0.02)
        for k in range(4):
            m.note(t0 + k * beat, beat * 0.8, f0, "pulse", 0.16, r=0.05)
        if bar >= 4:
            for k in range(4):
                m.drum(t0 + k * beat, "kick" if k % 2 == 0 else "snare", 0.32)
                m.drum(t0 + k * beat + beat / 2, "hat", 0.25)
        if 8 <= bar < 16:
            for k in range(4):
                m.note(t0 + k * beat, beat * 0.95, freq(melody[(bar * 4 + k) % 16]), "tri", 0.18, vibrato=0.004)
    return m


def village_fiber():
    """Mellow: sine pads, a walking bass and a soft bell melody, A minor, 84 BPM."""
    bpm, bars = 84, 14
    beat = 60 / bpm
    m = Mix(bars * 4 * beat + 3)
    prog = [("A2", True), ("F2", False), ("C3", False), ("G2", False)]
    bells = ["A5", "C6", "E6", "D6", "C6", "B5", "G5", "A5"]
    for bar in range(bars):
        root, minor = prog[bar % 4]
        f0 = freq(root)
        t0 = bar * 4 * beat
        for n in chord_notes(f0 * 2, minor):
            m.note(t0, 4 * beat, n, "sine", 0.07, a=0.6, r=0.8)
            m.note(t0, 4 * beat, n * 1.003, "tri", 0.03, a=0.6, r=0.8)
        for k, step in enumerate([0, 7, 12, 7]):
            m.note(t0 + k * beat, beat * 0.9, f0 * 2 ** (step / 12), "sine", 0.2, r=0.15)
        if bar >= 2:
            for k in range(2):
                m.note(t0 + k * 2 * beat + beat, beat * 1.5, freq(bells[(bar * 2 + k) % 8]), "sine", 0.1, a=0.002, r=1.0)
        if bar >= 6:
            for k in range(4):
                m.drum(t0 + k * beat + beat / 2, "hat", 0.12)
            m.drum(t0, "kick", 0.2)
    m.echo(beat * 0.75, 0.3)
    return m


def data_center_nights():
    """Synthwave: saw bass in eighths, a detuned saw lead, four-on-the-floor, E minor, 100 BPM."""
    bpm, bars = 100, 16
    beat = 60 / bpm
    m = Mix(bars * 4 * beat + 2)
    prog = [("E2", True), ("C2", False), ("G2", False), ("D2", False)]
    lead = ["B4", "E5", "G5", "F#5", "E5", "D5", "B4", "D5"]
    for bar in range(bars):
        root, minor = prog[bar % 4]
        f0 = freq(root)
        t0 = bar * 4 * beat
        for k in range(8):
            m.note(t0 + k * beat / 2, beat / 2 * 0.85, f0 * (2 if k % 2 else 1), "saw", 0.12, r=0.03)
        for n in chord_notes(f0 * 4, minor):
            m.note(t0, 4 * beat, n, "saw", 0.025, a=0.3, r=0.5)
        if bar >= 2:
            for k in range(4):
                m.drum(t0 + k * beat, "kick", 0.33)
                m.drum(t0 + k * beat + beat / 2, "hat", 0.2)
            m.drum(t0 + beat, "snare", 0.25)
            m.drum(t0 + 3 * beat, "snare", 0.25)
        if 4 <= bar < 14:
            for k in range(2):
                f = freq(lead[(bar * 2 + k) % 8])
                m.note(t0 + k * 2 * beat, beat * 1.9, f, "saw", 0.07, a=0.02, r=0.3, vibrato=0.006)
                m.note(t0 + k * 2 * beat, beat * 1.9, f * 1.006, "saw", 0.05, a=0.02, r=0.3)
    m.echo(beat / 2, 0.25)
    return m


TRACKS = [
    ("01-redstone-sunrise", "Redstone Sunrise", "Chiptune", redstone_sunrise, ((255, 120, 40), (90, 20, 20))),
    ("02-village-fiber", "Village Fiber", "Ambient", village_fiber, ((60, 160, 140), (20, 40, 70))),
    ("03-data-center-nights", "Data Center Nights", "Synthwave", data_center_nights, ((220, 60, 200), (20, 10, 60))),
]


def png(w, h, rows):
    raw = b"".join(b"\0" + bytes(r) for r in rows)

    def chunk(t, d):
        return struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d) & 0xFFFFFFFF)

    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b"")


def cover(top, bottom, seed):
    """A 160x160 sunset: a vertical gradient, a striped sun, a grid floor."""
    rnd = random.Random(seed)
    s = 160
    rows = []
    stars = {(rnd.randrange(s), rnd.randrange(70)) for _ in range(40)}
    for y in range(s):
        row = []
        k = y / (s - 1)
        base = [int(top[i] * (1 - k) + bottom[i] * k) for i in range(3)]
        for x in range(s):
            c = list(base)
            dx, dy = x - 80, y - 88
            if dx * dx + dy * dy < 46 * 46 and y < 100 and not (y > 70 and (y // 4) % 2 == 0):
                c = [255, 220 - (y - 42) * 2, 90]
            if y >= 100:
                c = [int(v * 0.35) for v in base]
                if (y - 100) % max(2, (y - 100) // 6 + 2) == 0 or (x - 80) * (y - 96) % 23 == 0 and abs(x - 80) < (y - 96) * 3:
                    c = [min(255, top[0]), min(255, top[1]), min(255, top[2])]
            if (x, y) in stars:
                c = [255, 255, 255]
            row += [max(0, min(255, v)) for v in c]
        rows.append(row)
    return png(s, s, rows)


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory() as tmp:
        for i, (file, title, genre, compose, colors) in enumerate(TRACKS, 1):
            print(f"composing {title}...")
            pcm = compose().pcm()
            wav = Path(tmp) / f"{file}.wav"
            with open(wav, "wb") as f:
                f.write(b"RIFF" + struct.pack("<I", 36 + len(pcm)) + b"WAVEfmt " + struct.pack("<IHHIIHH", 16, 1, 1, RATE, RATE * 2, 2, 16)
                        + b"data" + struct.pack("<I", len(pcm)) + pcm)
            art = Path(tmp) / f"{file}.png"
            art.write_bytes(cover(colors[0], colors[1], i))
            out = OUT / f"{file}.mp3"
            subprocess.run(["ffmpeg", "-y", "-loglevel", "error", "-i", str(wav), "-i", str(art), "-map", "0:a", "-map", "1:v",
                            "-c:a", "libmp3lame", "-b:a", "64k", "-ac", "1", "-c:v", "mjpeg", "-q:v", "4", "-disposition:v", "attached_pic",
                            "-id3v2_version", "3", "-metadata", f"title={title}", "-metadata", f"artist={ARTIST}",
                            "-metadata", f"album={ALBUM}", "-metadata", f"album_artist={ARTIST}", "-metadata", f"date={YEAR}",
                            "-metadata", f"genre={genre}", "-metadata", f"track={i}/{len(TRACKS)}",
                            "-metadata", "copyright=CC0 1.0 (public domain) - EvansComputerMod",
                            "-metadata:s:v", "title=Album cover", "-metadata:s:v", "comment=Cover (front)", str(out)], check=True)
            print(f"  {out} ({out.stat().st_size} bytes)")


if __name__ == "__main__":
    main()
