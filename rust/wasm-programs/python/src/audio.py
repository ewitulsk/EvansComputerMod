"""Speaker audio: the /dev/audio device of a Speaker next to the computer.

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
"""

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


class Speaker:
    def __init__(self, side=None):
        suffix = "" if side is None else "." + side
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

    def flush(self):
        """Drop whatever is queued."""
        self.control("flush")

    def status(self):
        import builtins
        with builtins.open(self._ctl_path, "rb", buffering=0) as ctl:
            text = ctl.read().decode()
        out = {}
        for line in text.splitlines():
            parts = line.split()
            if len(parts) == 2:
                out[parts[0]] = int(parts[1])
        return out

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
    """The speaker attached as `side` ('left', 'top', ...), or the first one."""
    return Speaker(side)
