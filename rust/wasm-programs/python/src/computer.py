"""This computer: its number, its label and its clocks. The Python
counterpart of Lua's `computer` module (same functions; Lua's os.time,
os.day, os.epoch, os.clock and os.getComputerID are built on them).

    import computer
    computer.id()              # 0, 1, 2 ...: this computer's number in the world
    computer.set_label("base") # kept in /etc/label
    computer.time()            # in-game hour, 0 <= t < 24 (6.0 at a world's first morning)
    computer.time("utc")       # the real hour, UTC ("local" is UTC too)
    computer.day()             # in-game day, 1 on the first day ("utc": days since 1970)
    computer.epoch("utc")      # real milliseconds since 1970 (in-game: 3600 per tick)
    computer.uptime()          # seconds since the program started
"""

import _console


def id():
    return _console.id()


def label():
    return _console.label()


def set_label(label=None):
    if label is not None and not isinstance(label, str):
        raise TypeError("label must be a str or None")
    _console.set_label(label)


def uptime():
    return _console.uptime()


def time(locale=None):
    return _console.time(locale)


def day(locale=None):
    return _console.day(locale)


def epoch(locale=None):
    return _console.epoch(locale)
