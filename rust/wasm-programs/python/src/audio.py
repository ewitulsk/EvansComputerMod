"""Speaker and microphone audio: /dev/audio of a Speaker and /dev/mic of a
Microphone next to the computer (or the ePhone's own).

    import audio

    spk = audio.open()                  # the first speaker; audio.open("left") for a side
    spk.tone(440, 0.25)                 # a quarter second of A4 (square wave)
    spk.notes("C4 E4 G4 C5", 0.15)      # note names, each 0.15 s; "-" is a rest
    spk.set_format(rate=8000, bits=8, channels=1)
    spk.write(pcm_bytes)                # raw PCM in the current format (waits for room)
    print(spk.status())                 # {'rate': 8000, ..., 'buffered_ms': 40, 'underruns': 0}
    spk.close()

The speaker plays samples at its rate; writes wait while more than `latency`
ms of audio is queued, so a loop that just writes stays in time with the
music. Formats: rate 8000-48000 Hz, 8-bit unsigned or 16-bit signed
little-endian, mono or stereo (mixed down: a speaker is one point source).

    mic = audio.open_mic()              # the default microphone; open_mic("top") for a side
    mic.set_format(rate=16000)          # 16-bit mono by default
    pcm = mic.record(2.0)               # two seconds of 16-bit PCM (waits while it records)
    print(audio.peak_db(pcm))           # loudest sample, dBFS (-99 = silence)
    audio.save_wav("/home/memo.wav", pcm, 16000)
    print(mic.status())                 # {'rate': 16000, 'gain': 50, 'overruns': 0, 'level_db': -23, ...}
    mic.close()

A microphone hears players talking and speakers playing around it, quieter
with distance. Capture starts with the first read and runs on its own clock:
a read waits until the audio it asks for has been captured, and a program
that reads less than once a second loses audio (an overrun). Gain 0-100:
50 is the capsule's own level, every 25 is 10 dB, 0 mutes.

With no side given, open() and open_mic() use the default devices chosen in
Settings > Sound (/etc/sound.conf), else the first speaker / microphone.
"""

SOUND_CONF = "/etc/sound.conf"
MIXER = "/dev/mixer"
MAX_APP_PERCENT = 150
ROLES = ("media", "voice", "phone")

TONE_RATE = 8000
_NOTES = {"C": -9, "D": -7, "E": -5, "F": -4, "G": -2, "A": 0, "B": 2}


def note_freq(name):
    """'A4' -> 440.0, 'C#5', 'Eb3' ..."""
    letter = name[0].upper()
    i = 1
    semis = _NOTES[letter]
    while i < len(name) and name[i] in "#b":
        semis += 1 if name[i] == "#" else -1
        i += 1
    octave = int(name[i:]) if i < len(name) else 4
    return 440.0 * 2 ** ((semis + 12 * (octave - 4)) / 12.0)


def default_device(kind):
    """The side Settings > Sound chose for kind 'output' (speaker) or 'input' (microphone), or None."""
    import builtins
    try:
        with builtins.open(SOUND_CONF, "r") as f:
            text = f.read()
    except OSError:
        return None
    for line in text.splitlines():
        parts = line.split()
        if len(parts) >= 2 and parts[0] == kind:
            return parts[1]
    return None


def _side_for(side, kind, prefix):
    """The device suffix for an explicit side, else the default if its device exists, else ''."""
    if side is not None:
        return "." + side
    d = default_device(kind)
    if d:
        try:
            import os
            os.stat(prefix + "." + d)
            return "." + d
        except ImportError:
            return "." + d
        except OSError:
            pass
    return ""


def _ctl_status(path):
    import builtins
    with builtins.open(path, "rb", buffering=0) as ctl:
        text = ctl.read().decode()
    out = {}
    for line in text.splitlines():
        parts = line.split()
        if len(parts) == 2:
            try:
                out[parts[0]] = int(parts[1])
            except ValueError:
                pass
    return out


def peak_db(pcm):
    """Loudest sample of 16-bit little-endian PCM bytes, in dBFS (-99 for silence)."""
    peak = 0
    for i in range(0, len(pcm) - 1, 2):
        v = pcm[i] | (pcm[i + 1] << 8)
        if v >= 32768:
            v -= 65536
        if abs(v) > peak:
            peak = abs(v)
    if peak == 0:
        return -99
    r = peak / 32768.0
    # round(20 * log10(r)) without the math module: the first whole dB whose half-step floor r reaches.
    for db in range(0, -99, -1):
        if r >= 10 ** ((db - 0.5) / 20.0):
            return db
    return -99


def wav_header(rate, bits, channels, data_len):
    """A 44-byte RIFF/WAVE header for data_len bytes of PCM."""
    block = channels * bits // 8

    def le(v, n):
        return bytes((v >> (8 * i)) & 0xFF for i in range(n))
    return (b"RIFF" + le(36 + data_len, 4) + b"WAVEfmt " + le(16, 4) + le(1, 2) + le(channels, 2)
            + le(rate, 4) + le(rate * block, 4) + le(block, 2) + le(bits, 2) + b"data" + le(data_len, 4))


def save_wav(path, pcm, rate, bits=16, channels=1):
    """Write PCM bytes as a WAV file."""
    import builtins
    with builtins.open(path, "wb") as f:
        f.write(wav_header(rate, bits, channels, len(pcm)) + bytes(pcm))


class Speaker:
    def __init__(self, side=None):
        suffix = _side_for(side, "output", "/dev/audio")
        self.device = "/dev/audio" + suffix
        import builtins
        self._pcm = builtins.open(self.device, "wb", buffering=0)
        self._ctl_path = "/dev/audioctl" + suffix
        self._format = None

    def control(self, command):
        """Send one /dev/audioctl command, e.g. 'volume 50' or 'flush'."""
        import builtins
        with builtins.open(self._ctl_path, "wb", buffering=0) as ctl:
            ctl.write((command + "\n").encode())

    def set_format(self, rate=48000, bits=16, channels=1):
        self.control("rate %d" % rate)
        self.control("bits %d" % bits)
        self.control("channels %d" % channels)
        self._format = (rate, bits, channels)

    def set_volume(self, volume):
        self.control("volume %d" % volume)

    def set_role(self, role):
        """Who this stream is for: "media" (default), "voice" (a call) or "phone"
        (a cellular call, a ringtone). While a stream carries sound, every stream
        with a lower role is ducked (media < voice < phone)."""
        if role not in ROLES:
            raise ValueError("role is media, voice or phone, not %r" % (role,))
        self.control("role " + role)

    def ducked(self):
        """True while a louder, higher-role stream lowers this one."""
        return self.status().get("ducked", 0) == 1

    def flush(self):
        """Drop whatever is queued."""
        self.control("flush")

    def status(self):
        return _ctl_status(self._ctl_path)

    def write(self, pcm):
        """Raw PCM bytes in the current format."""
        self._pcm.write(pcm)

    def tone(self, freq, seconds, volume=0.5, wave="square"):
        """Play a tone (8 kHz, 8-bit). wave: square, triangle or saw."""
        if self._format != (TONE_RATE, 8, 1):
            self.set_format(TONE_RATE, 8, 1)
        n = int(TONE_RATE * seconds)
        amp = int(max(0.0, min(1.0, volume)) * 100)
        if freq <= 0:
            self.write(bytes([128]) * n)
            return
        period = TONE_RATE / float(freq)
        out = bytearray(n)
        for i in range(n):
            ph = (i % period) / period
            if wave == "triangle":
                v = 1 - 4 * abs(ph - 0.5)
            elif wave == "saw":
                v = 2 * ph - 1
            else:
                v = 1 if ph < 0.5 else -1
            out[i] = 128 + int(v * amp)
        self.write(bytes(out))

    def notes(self, melody, seconds=0.2, volume=0.5, wave="square"):
        """Play note names separated by spaces ('C4 E4 G4 -'); '-' is a rest."""
        for name in melody.split():
            f = 0 if name == "-" else note_freq(name)
            self.tone(f, seconds * 0.9, volume, wave)
            self.tone(0, seconds * 0.1)

    def close(self):
        self._pcm.close()

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        self.close()


def open(side=None):
    """The speaker attached as `side` ('left', 'top', ...), or the default / first one."""
    return Speaker(side)


def open_speaker(side=None):
    """The same as open(): a speaker stream of its own."""
    return Speaker(side)


def _meter(peak):
    """A peak (0-32767) as a meter fill 0-100 on a dB scale from -60 dBFS
    (round((20 * log10(peak / 32767) + 60) / 0.6), without the math module)."""
    if peak <= 0:
        return 0
    for m in range(100, 0, -1):
        if peak >= 32767 * 10 ** (((m - 0.5) * 0.6 - 60) / 20.0):
            return m
    return 0


class Mixer:
    """The Volume Mixer (/dev/mixer): every app's strip, each speaker's master
    strip and the microphones. Made by audio.mixer()."""

    DEVICE = MIXER

    def strips(self):
        """Every strip as a dict with 'kind' 'output', 'app' or 'input'."""
        import builtins
        with builtins.open(self.DEVICE, "rb", buffering=0) as f:
            text = f.read().decode()
        return parse_mixer(text)

    def _send(self, cmd):
        import builtins
        try:
            with builtins.open(self.DEVICE, "wb", buffering=0) as f:
                f.write((cmd + "\n").encode())
        except PermissionError:
            raise PermissionError("changing the Volume Mixer needs the mixer permission")
        except OSError as e:
            raise ValueError("the Volume Mixer refused %r (%s)" % (cmd, e))

    def set(self, target, percent):
        """An app's volume 0-150 %, or a speaker's ('master', 'master.left') 0-100 %."""
        percent = int(percent)
        top = 100 if _is_master(target) else MAX_APP_PERCENT
        if percent < 0 or percent > top:
            raise ValueError("%s's volume is 0-%d" % ("a speaker" if top == 100 else "an app", top))
        self._send("set %s %d" % (target, percent))

    def mute(self, target, on=True):
        """Mute (or with on=False unmute) an app or a speaker."""
        self._send("mute %s %s" % (target, "on" if on else "off"))

    def route(self, app, side):
        """Play an app on the speaker attached as `side`; None (or '-') = the default speaker."""
        if _is_master(app):
            raise ValueError("route moves an app; a speaker can't be routed")
        self._send("route %s %s" % (app, "-" if side in (None, "", "-", "default") else side))

    def gain(self, input, n):
        """A microphone's gain 0-100 by its side."""
        n = int(n)
        if n < 0 or n > 100:
            raise ValueError("gain is 0-100")
        self._send("gain %s %d" % (input, n))

    def reset(self, app):
        """Forget an app's volume, mute and route."""
        self._send("reset %s" % app)


def mixer():
    """The Volume Mixer: mixer().strips(), .set(target, percent), .mute(target, on=True),
    .route(app, side), .gain(input, n), .reset(app)."""
    return Mixer()


def _is_master(target):
    return target == "master" or target.startswith("master.")


def parse_mixer(text):
    """/dev/mixer's text as a list of strip dicts."""
    out = []
    for line in text.splitlines():
        w = line.split()
        if len(w) < 2 or w[0] not in ("output", "app", "input"):
            continue
        kv = {}
        for item in w[2:]:
            if "=" in item:
                k, v = item.split("=", 1)
                kv[k] = v

        def num(k, d=0):
            try:
                return int(kv.get(k, d))
            except ValueError:
                return d

        def side(k):
            v = kv.get(k, "-")
            return None if v in ("", "-") else v
        if w[0] == "output":
            peak = num("peak")
            out.append({"kind": "output", "side": w[1], "target": "master." + w[1], "volume": num("volume"),
                        "mute": num("mute") == 1, "peak": peak, "meter": _meter(peak),
                        "label": kv.get("label", "").replace("_", " ")})
        elif w[0] == "app":
            peak = num("peak")
            out.append({"kind": "app", "app": w[1], "target": w[1], "volume": num("volume"), "mute": num("mute") == 1,
                        "route": side("route"), "on": side("on"), "peak": peak, "meter": _meter(peak),
                        "ducked": num("ducked") == 1, "role": kv.get("role", "media"), "streams": num("streams")})
        else:
            level = num("level", -99)
            out.append({"kind": "input", "side": w[1], "target": w[1], "gain": num("gain"), "level": level,
                        "meter": max(0, min(60, level + 60)) * 100 // 60})
    return out


class Microphone:
    def __init__(self, side=None):
        suffix = _side_for(side, "input", "/dev/mic")
        self.device = "/dev/mic" + suffix
        import builtins
        self._pcm = builtins.open(self.device, "rb", buffering=0)
        self._ctl_path = "/dev/micctl" + suffix
        st = self.status()
        self.rate = st.get("rate", 48000)
        self.bits = st.get("bits", 16)
        self.channels = st.get("channels", 1)

    def control(self, command):
        """Send one /dev/micctl command, e.g. 'gain 60' or 'flush'."""
        import builtins
        with builtins.open(self._ctl_path, "wb", buffering=0) as ctl:
            ctl.write((command + "\n").encode())

    def set_format(self, rate=16000, bits=16, channels=1):
        self.control("rate %d" % rate)
        self.control("bits %d" % bits)
        self.control("channels %d" % channels)
        self.rate, self.bits, self.channels = rate, bits, channels

    def set_gain(self, gain):
        """Input volume 0-100 (50 = the capsule's own level, 0 = mute)."""
        self.control("gain %d" % gain)

    def flush(self):
        """Drop what was captured and not read yet."""
        self.control("flush")

    def status(self):
        return _ctl_status(self._ctl_path)

    def level(self):
        """Peak of the last read, dBFS (-99 before the first)."""
        return self.status().get("level_db", -99)

    def read(self, nbytes):
        """Raw PCM in the current format: waits until nbytes (whole frames, at most half a second) are captured."""
        return self._pcm.read(nbytes)

    def record(self, seconds):
        """`seconds` of audio as PCM bytes in the current format (waits while it records)."""
        frame = self.bits // 8 * self.channels
        want = int(round(seconds * self.rate)) * frame
        chunk = max(frame, self.rate // 10 * frame)
        out = bytearray()
        while len(out) < want:
            got = self._pcm.read(min(chunk, want - len(out)))
            if not got:
                break
            out += got
        return bytes(out)

    def close(self):
        self._pcm.close()

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        self.close()


def open_mic(side=None):
    """The microphone attached as `side` ('top', ...; the ePhone's is 'microphone'), or the default / first one."""
    return Microphone(side)
