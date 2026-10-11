"""Software radio flowgraphs: SDR blocks, demodulators, modems and sinks.

    import radio
    sdr = radio.open("sdr_0")              # or an attachment name, or an index
    sdr.tune(146.52e6, rate=48_000)
    fg = radio.Flowgraph(sdr >> radio.fm_demod(5e3) >> radio.lowpass(3e3) >> radio.speaker())
    fg.run(seconds=10)

Streams are complex baseband, real audio, or frames (packet bodies). The
heavy per-sample work runs natively (`_radio`, ecm-dsp); this module only
wires blocks together. Samples travel as `radio.Samples` (packed float32).

Sources: radio.open(...) (an SDR), tone(), file_source() (.cf32/.cs16 with
SigMF metadata), wav_source(), frames_source().
Blocks:  fm_demod, wbfm_demod, fm_mod, am_demod, am_mod, ssb_demod(mode),
         ssb_mod(mode), lowpass, resample, decimate, agc, squelch, shift, gain,
         dc_block, real, to_complex, afsk1200, fsk, bpsk, chirp.
         Packet modems modulate when fed frames and demodulate when fed samples.
Sinks:   speaker(), wav_sink(path), iq_sink(path), collect(), frames(), or an
         SDR (transmit; call sdr.tx(True) first; closing it switches tx off).

Flowgraph.run() ends when the source ends, at a seconds=/samples= limit, after
`timeout` seconds without input (10 s by default: a silent SDR), or on stop().
"""

import builtins
import _radio

try:
    import time as _time
except ImportError:  # pragma: no cover - every ECM build has time
    _time = None


class RadioError(Exception):
    pass


COMPLEX = "complex"
REAL = "real"
FRAMES = "frames"


class Samples:
    """A buffer of one stream kind: packed float32 bytes (complex: I,Q pairs)
    or, for frames, a list of bytes."""

    def __init__(self, kind, data, rate):
        self.kind = kind
        self.data = data
        self.rate = rate

    def __len__(self):
        if self.kind == FRAMES:
            return len(self.data)
        return len(self.data) // (8 if self.kind == COMPLEX else 4)

    def to_list(self):
        """Python complex numbers (complex), floats (real) or bytes (frames)."""
        if self.kind == FRAMES:
            return list(self.data)
        return _radio.to_list(self.kind, self.data)

    def power_db(self):
        return _radio.power_db(self.kind, self.data)

    def __repr__(self):
        return "<Samples %s x%d @ %g>" % (self.kind, len(self), self.rate)

    @staticmethod
    def from_list(values, rate, kind=COMPLEX):
        return Samples(kind, _radio.from_list(kind, list(values)), rate)


def _as_samples(x, rate, kind=COMPLEX):
    if isinstance(x, Samples):
        return x
    if isinstance(x, (bytes, bytearray)):
        return Samples(kind, bytes(x), rate)
    return Samples.from_list(x, rate, kind)


# ---------------------------------------------------------------- pipelines

class _Node:
    def __rshift__(self, other):
        return Pipeline([self]) >> other


class Pipeline:
    """`a >> b >> c`: a source, blocks, and optionally a sink."""

    def __init__(self, items):
        self.items = list(items)

    def __rshift__(self, other):
        if isinstance(other, Pipeline):
            return Pipeline(self.items + other.items)
        return Pipeline(self.items + [other])

    def __repr__(self):
        return " >> ".join(repr(i) for i in self.items)


class Block(_Node):
    """A processing block; built natively once its input rate is known."""

    def __init__(self, name, **params):
        self.name = name
        self.params = dict((k, v) for k, v in params.items() if v is not None)

    def spec(self):
        return (self.name, self.params)

    def __repr__(self):
        args = ", ".join("%s=%r" % kv for kv in sorted(self.params.items()))
        return "%s(%s)" % (self.name, args)


class Chain:
    """Blocks built for an input kind and rate; `process(samples)` runs them."""

    def __init__(self, blocks, kind, rate):
        self.handle, self.kind, self.rate, self.description = _radio.chain_new(
            [b.spec() for b in blocks], kind, float(rate))
        self.in_kind = kind

    def process(self, s):
        if s.kind != self.in_kind:
            raise RadioError("chain expects %s samples, got %s" % (self.in_kind, s.kind))
        return Samples(self.kind, _radio.chain_process(self.handle, s.kind, s.data), self.rate)

    def close(self):
        if self.handle is not None:
            _radio.chain_free(self.handle)
            self.handle = None


# ---------------------------------------------------------------- blocks

def fm_demod(deviation=5e3, tau=0.0):
    """FM discriminator (complex -> real). `tau`: de-emphasis, e.g. 75e-6."""
    return Block("fm_demod", deviation=deviation, tau=tau)


def wbfm_demod(deviation=75e3, tau=75e-6):
    return Block("fm_demod", deviation=deviation, tau=tau)


def fm_mod(deviation=5e3, tau=0.0):
    """FM modulator (real -> complex)."""
    return Block("fm_mod", deviation=deviation, tau=tau)


def am_demod():
    return Block("am_demod")


def am_mod(index=0.8):
    return Block("am_mod", index=index)


def ssb_demod(mode="usb"):
    return Block("ssb_demod", mode=mode)


def ssb_mod(mode="usb"):
    return Block("ssb_mod", mode=mode)


def lowpass(cutoff):
    return Block("lowpass", cutoff=cutoff)


def resample(rate):
    return Block("resample", rate=rate)


def decimate(factor):
    return Block("decimate", factor=factor)


def agc(target=0.5):
    return Block("agc", target=target)


def squelch(threshold=-40.0):
    """Zero a complex stream while its power is below `threshold` dBFS."""
    return Block("squelch", threshold=threshold)


def shift(hz):
    return Block("shift", hz=hz)


def gain(g):
    return Block("gain", gain=g)


def dc_block():
    return Block("dc_block")


def real():
    return Block("real")


def to_complex():
    return Block("complex")


def afsk1200():
    """Bell 202 AX.25 packet modem (audio). Frames in -> audio; audio in -> frames."""
    return Block("afsk1200")


def fsk(baud=1200, deviation=2400):
    return Block("fsk", baud=baud, deviation=deviation)


def bpsk(baud=2400):
    return Block("bpsk", baud=baud)


def chirp(sf=7, bw=12000):
    return Block("chirp", sf=sf, bw=bw)


# ---------------------------------------------------------------- sources

class Source(_Node):
    kind = COMPLEX
    rate = 48000.0

    def read(self, n):
        """Up to n samples, or None at the end of the stream."""
        raise NotImplementedError

    def close(self):
        pass


class SDR(Source):
    """An SDR block attached to the computer (/dev/sdr*). As a source it
    receives; as a sink it transmits (after tx(True))."""

    def __init__(self, name="", data_path=None, ctl_path=None):
        if data_path is None or ctl_path is None:
            d, c = _radio.resolve_sdr(str(name))
            data_path = data_path or d
            ctl_path = ctl_path or c
        self.data_path = data_path
        self.ctl_path = ctl_path
        self._rx = None
        self._tx = None
        self._sent = 0
        self._tx_start = None
        self._keyed = False
        self.lead = 0.25
        try:
            st = self.status()
        except OSError as e:
            raise RadioError("no SDR at %s: %s" % (ctl_path, e))
        self.format = st.get("format", "cs16")
        self.rate = float(st.get("rate", 48000))
        self.freq = float(st.get("freq", 0))

    def control(self, command):
        with builtins.open(self.ctl_path, "wb", buffering=0) as f:
            f.write((command.strip() + "\n").encode())

    def status(self):
        with builtins.open(self.ctl_path, "rb", buffering=0) as f:
            text = b""
            while True:
                chunk = f.read(4096)
                if not chunk:
                    break
                text += chunk
        return _radio.parse_status(text.decode())

    def tune(self, freq, rate=None, bw=None):
        freq = _radio.parse_freq(freq)
        self.control("freq %d" % round(freq))
        self.freq = freq
        if rate is not None:
            self.set_rate(rate)
        if bw is not None:
            self.control("bw %d" % round(_radio.parse_freq(bw)))
        return self

    def set_rate(self, rate):
        rate = int(round(_radio.parse_freq(rate)))
        self.control("rate %d" % rate)
        self.rate = float(rate)
        self._tx_start = None

    def gain(self, db):
        self.control("gain %s" % db)

    def agc(self, on=True):
        self.control("gain agc" if on else "agc 0")

    def set_format(self, fmt):
        self.control("format %s" % fmt)
        self.format = fmt

    def tx(self, on=True, power=None):
        """Transmitter on (at `power` dBm, default the tier maximum) or off."""
        if on:
            self.control("tx on" if power is None else "tx on %s" % power)
        else:
            self.control("tx off")
        self._keyed = bool(on)
        self._tx_start = None
        self._sent = 0

    def timestamp(self):
        return int(self.status().get("timestamp", 0))

    def read(self, n=4096):
        if self._rx is None:
            self._rx = builtins.open(self.data_path, "rb", buffering=0)
        size = _radio.sample_size(self.format)
        raw = self._rx.read(n * size) or b""
        raw = raw[:len(raw) - len(raw) % size]
        return Samples(COMPLEX, _radio.decode(self.format, raw), self.rate)

    def prepare(self, kind, rate):
        if kind != COMPLEX:
            raise RadioError("an SDR transmits complex samples, got %s" % kind)
        if abs(rate - self.rate) > 0.5:
            return [resample(self.rate)]
        return []

    def write(self, samples):
        s = _as_samples(samples, self.rate)
        if s.kind != COMPLEX:
            raise RadioError("an SDR transmits complex samples")
        if self._tx is None:
            self._tx = builtins.open(self.data_path, "wb", buffering=0)
        self._tx.write(_radio.encode(self.format, s.data))
        n = len(s)
        now = self.timestamp()
        if self._tx_start is None:
            self._tx_start = now
        self._sent += n
        ahead = self._sent - (now - self._tx_start)
        wait = (ahead - self.lead * self.rate) / self.rate
        if wait > 0 and _time is not None:
            _time.sleep(wait)

    def close(self):
        """Close the device files. A transmitting SDR first lets its queued
        samples go out (at most lead + 1 s) and then switches tx off."""
        try:
            if self._keyed:
                if self._tx_start is not None and _time is not None:
                    left = (self._sent - (self.timestamp() - self._tx_start)) / self.rate
                    if left > 0:
                        _time.sleep(min(left, self.lead + 1.0))
                self.tx(False)
        finally:
            for f in (self._rx, self._tx):
                if f is not None:
                    f.close()
            self._rx = self._tx = None


def open(name="", **kw):
    """The SDR called `name`: '' (first), 'sdr_0' / 0 (index), or an
    attachment name ('left', ...)."""
    return SDR(name, **kw)


class ToneSource(Source):
    def __init__(self, freq, rate=48000, amplitude=0.5, seconds=None, kind=COMPLEX):
        self.freq = float(freq)
        self.rate = float(rate)
        self.amplitude = float(amplitude)
        self.kind = kind
        self.left = None if seconds is None else int(seconds * rate)
        self.phase = 0.0

    def read(self, n):
        if self.left is not None:
            if self.left <= 0:
                return None
            n = min(n, self.left)
            self.left -= n
        data, self.phase = _radio.tone(self.freq, self.rate, n, self.amplitude, self.phase, self.kind)
        return Samples(self.kind, data, self.rate)


def tone(freq, rate=48000, amplitude=0.5, seconds=None, kind=COMPLEX):
    """A tone at `freq` Hz (complex: e^{jwt}; real: cos)."""
    return ToneSource(freq, rate, amplitude, seconds, kind)


def _meta_path(path):
    slash = path.rfind("/") + 1
    dot = path.rfind(".")
    base = path[:dot] if dot >= slash else path
    return base + ".sigmf-meta"


def _read_file(path):
    with builtins.open(path, "rb") as f:
        return f.read()


class FileSource(Source):
    """IQ from a .cf32 / .cs16 / .sigmf-data file; rate and frequency from
    the .sigmf-meta beside it when present."""

    def __init__(self, path, fmt=None, rate=None, repeat=False):
        meta = None
        try:
            meta = _radio.sigmf_parse(_read_file(_meta_path(path)).decode())
        except (OSError, ValueError):
            meta = None
        if fmt is None:
            lower = path.lower()
            if meta is not None:
                fmt = meta["datatype"]
            elif lower.endswith(".cs16") or lower.endswith(".ci16"):
                fmt = "cs16"
            else:
                fmt = "cf32"
        if rate is None:
            if meta is None:
                raise RadioError("%s has no .sigmf-meta: pass rate=" % path)
            rate = meta["rate"]
        self.meta = meta
        self.freq = meta["freq"] if meta else 0.0
        self.rate = float(rate)
        self.kind = COMPLEX
        self.format = fmt
        self.repeat = repeat
        self.data = _radio.decode(fmt, _read_file(path))
        self.pos = 0

    def read(self, n):
        if self.pos >= len(self.data):
            if not self.repeat or not self.data:
                return None
            self.pos = 0
        chunk = self.data[self.pos:self.pos + 8 * n]
        self.pos += len(chunk)
        return Samples(COMPLEX, chunk, self.rate)


def file_source(path, fmt=None, rate=None, repeat=False):
    return FileSource(path, fmt, rate, repeat)


class WavSource(Source):
    def __init__(self, path, repeat=False, raw_rate=8000):
        rate, self.data = _radio.read_audio_file(path, _read_file(path), raw_rate)
        self.rate = float(rate)
        self.kind = REAL
        self.repeat = repeat
        self.pos = 0

    def read(self, n):
        if self.pos >= len(self.data):
            if not self.repeat or not self.data:
                return None
            self.pos = 0
        chunk = self.data[self.pos:self.pos + 4 * n]
        self.pos += len(chunk)
        return Samples(REAL, chunk, self.rate)


def wav_source(path, repeat=False, raw_rate=8000):
    """Audio from a WAV file (or headerless 16-bit PCM at raw_rate)."""
    return WavSource(path, repeat, raw_rate)


class FramesSource(Source):
    def __init__(self, frames, rate=48000, gap=0.1):
        self.frames = [f.encode() if isinstance(f, str) else bytes(f) for f in frames]
        self.rate = float(rate)
        self.kind = FRAMES

    def read(self, n):
        if not self.frames:
            return None
        batch, self.frames = self.frames[:1], self.frames[1:]
        return Samples(FRAMES, batch, self.rate)


def frames_source(frames, rate=48000):
    """Packet bodies to modulate (bytes, or str encoded as UTF-8), one per read.
    `rate` is the sample rate the modulator will produce."""
    return FramesSource(frames, rate)


# ---------------------------------------------------------------- sinks

class Sink(_Node):
    def prepare(self, kind, rate):
        """Check the incoming stream; may return blocks to append (e.g. a resampler)."""
        return []

    def write(self, samples):
        raise NotImplementedError

    def close(self):
        pass


class SpeakerSink(Sink):
    def __init__(self, side=None, volume=None, rate=None):
        suffix = "" if side is None else "." + side
        self.device = "/dev/audio" + suffix
        self.ctl = "/dev/audioctl" + suffix
        self.volume = volume
        self.want_rate = rate
        self._pcm = None

    def _control(self, command):
        with builtins.open(self.ctl, "wb", buffering=0) as f:
            f.write((command + "\n").encode())

    def prepare(self, kind, rate):
        if kind != REAL:
            raise RadioError("speaker() plays real audio, got %s (add a demodulator)" % kind)
        extra = []
        target = self.want_rate
        if target is None:
            target = rate if 8000 <= rate <= 48000 and abs(rate - round(rate)) < 1e-6 else 24000
        if abs(target - rate) > 0.5:
            extra.append(resample(target))
        self.rate = int(round(target))
        self._pcm = builtins.open(self.device, "wb", buffering=0)
        self._control("rate %d" % self.rate)
        self._control("bits 16")
        self._control("channels 1")
        if self.volume is not None:
            self._control("volume %d" % self.volume)
        return extra

    def write(self, s):
        if len(s):
            self._pcm.write(_radio.pcm16(s.data))

    def close(self):
        if self._pcm is not None:
            self._pcm.close()
            self._pcm = None


def speaker(side=None, volume=None, rate=None):
    """Play real audio on the Speaker (/dev/audio). Resamples when needed."""
    return SpeakerSink(side, volume, rate)


class WavSink(Sink):
    def __init__(self, path):
        self.path = path
        self.chunks = []
        self.rate = 0

    def prepare(self, kind, rate):
        if kind != REAL:
            raise RadioError("wav_sink() records real audio, got %s" % kind)
        self.rate = int(round(rate))
        return []

    def write(self, s):
        self.chunks.append(_radio.pcm16(s.data))

    def close(self):
        pcm = b"".join(self.chunks)
        with builtins.open(self.path, "wb") as f:
            f.write(_radio.wav_header(self.rate, len(pcm) // 2))
            f.write(pcm)


def wav_sink(path):
    return WavSink(path)


class IqSink(Sink):
    def __init__(self, path, fmt=None, freq=0.0, description="", meta=True):
        lower = path.lower()
        if fmt is None:
            fmt = "cs16" if (lower.endswith(".cs16") or lower.endswith(".ci16")) else "cf32"
        self.path = path
        self.format = fmt
        self.freq = float(freq)
        self.description = description
        self.meta = meta
        self.rate = 0.0
        self._f = None

    def prepare(self, kind, rate):
        if kind != COMPLEX:
            raise RadioError("iq_sink() records complex samples, got %s" % kind)
        self.rate = rate
        self._f = builtins.open(self.path, "wb")
        return []

    def write(self, s):
        self._f.write(_radio.encode(self.format, s.data))

    def close(self):
        if self._f is not None:
            self._f.close()
            self._f = None
        if self.meta:
            text = _radio.sigmf_meta(self.format, self.rate, self.freq, self.description, None, "ecm radio.py")
            with builtins.open(_meta_path(self.path), "w") as f:
                f.write(text)


def iq_sink(path, fmt=None, freq=0.0, description=""):
    """Record complex samples (.cf32 / .cs16) plus a SigMF .sigmf-meta."""
    return IqSink(path, fmt, freq, description)


class Collect(Sink):
    """Keeps everything it receives: `.samples` (one Samples) or `.frames`."""

    def __init__(self, limit=None):
        self.limit = limit
        self.parts = []
        self.frames = []
        self.kind = None
        self.rate = 0.0

    def prepare(self, kind, rate):
        self.kind, self.rate = kind, rate
        return []

    def write(self, s):
        if s.kind == FRAMES:
            self.frames.extend(s.data)
        else:
            self.parts.append(s.data)

    @property
    def samples(self):
        return Samples(self.kind, b"".join(self.parts), self.rate)


def collect():
    return Collect()


class FramesSink(Sink):
    """Decoded packets: printed (AX.25 when it parses) and/or passed to `callback`."""

    def __init__(self, callback=None, echo=True):
        self.callback = callback
        self.echo = echo
        self.frames = []

    def prepare(self, kind, rate):
        if kind != FRAMES:
            raise RadioError("frames() takes decoded packets; put a modem (e.g. afsk1200()) in front")
        return []

    def write(self, s):
        for f in s.data:
            self.frames.append(f)
            if self.echo:
                d = _radio.ax25_decode(f)
                if d is not None:
                    print("%s>%s: %s" % (d["src"], d["dest"], d["text"]))
                else:
                    print("frame %d bytes: %r" % (len(f), f))
            if self.callback is not None:
                self.callback(f)


def frames(callback=None, echo=True):
    return FramesSink(callback, echo)


# ---------------------------------------------------------------- flowgraph

class Flowgraph:
    """Runs `source >> blocks... >> sink`. Without a sink, output is collected
    (see `.sink.samples` / `.sink.frames`)."""

    def __init__(self, pipeline, block_size=4096):
        if isinstance(pipeline, _Node):
            pipeline = Pipeline([pipeline])
        items = list(pipeline.items)
        if not items or not isinstance(items[0], Source):
            raise RadioError("a flowgraph starts with a source (an SDR, tone(), file_source(), ...)")
        self.source = items[0]
        rest = items[1:]
        if rest and isinstance(rest[-1], (Sink, SDR)):
            self.sink = rest[-1]
            rest = rest[:-1]
        else:
            self.sink = Collect()
        for b in rest:
            if not isinstance(b, Block):
                raise RadioError("%r can't be in the middle of a flowgraph" % (b,))
        self.blocks = rest
        self.block_size = block_size
        probe = Chain(self.blocks, self.source.kind, self.source.rate)
        extra = self.sink.prepare(probe.kind, probe.rate)
        if extra:
            probe.close()
            self.blocks = self.blocks + extra
            probe = Chain(self.blocks, self.source.kind, self.source.rate)
        self.chain = probe
        self.consumed = 0
        self.last_read = 0
        self.idle = False
        self._stop = False

    def describe(self):
        return self.chain.description

    def step(self):
        """Process one buffer; False when the source is exhausted.
        `.last_read` is how many samples the source gave (0 = nothing yet)."""
        s = self.source.read(self.block_size)
        if s is None:
            return False
        self.last_read = len(s)
        self.consumed += len(s)
        out = self.chain.process(s)
        if len(out):
            self.sink.write(out)
        return True

    def stop(self):
        """End run() after the current buffer (e.g. from a frames() callback)."""
        self._stop = True

    def run(self, seconds=None, samples=None, timeout=10.0):
        """Run until the source ends, `seconds` / `samples` of input, stop(), or
        `timeout` seconds in a row without input (a silent SDR's reads come
        back empty; `.idle` is then True). timeout=None waits forever."""
        limit = samples
        if seconds is not None:
            limit = int(seconds * self.source.rate)
        self._stop = False
        self.idle = False
        clock = getattr(_time, "monotonic", None) or getattr(_time, "time", None)
        quiet_since = None
        quiet_reads = 0
        try:
            while not self._stop and (limit is None or self.consumed < limit):
                if not self.step():
                    break
                if self.last_read:
                    quiet_since = None
                    quiet_reads = 0
                    continue
                if timeout is None:
                    continue
                quiet_reads += 1
                if clock is None:
                    # no clock: an empty SDR read takes ~2 s
                    if quiet_reads * 2.0 >= timeout:
                        self.idle = True
                        break
                    continue
                now = clock()
                if quiet_since is None:
                    quiet_since = now
                elif now - quiet_since >= timeout:
                    self.idle = True
                    break
                _time.sleep(0.01)
        finally:
            self.close()
        return self.sink

    def close(self):
        """Close the chain, the sink (an SDR sink stops transmitting) and the
        source (an SDR source's device file; the next read reopens it)."""
        try:
            self.chain.close()
        finally:
            try:
                self.sink.close()
            finally:
                if self.source is not self.sink:
                    self.source.close()


# ---------------------------------------------------------------- helpers

def ax25(dest, src, info, pid=0xF0):
    """An AX.25 UI frame body (for afsk1200())."""
    if isinstance(info, str):
        info = info.encode()
    return _radio.ax25_encode(dest, src, info, pid)


def parse_ax25(frame):
    """{'src', 'dest', 'digis', 'pid', 'info', 'text'} or None."""
    return _radio.ax25_decode(frame)


def find_signals(samples, center=0.0, nfft=1024, threshold=15.0):
    """[(freq, power_db, snr_db)] of signals in a complex block."""
    return _radio.find_signals(samples.data, samples.rate, center, nfft, threshold)


def parse_freq(value):
    return _radio.parse_freq(value)
