"""The terminal: a character grid with a cursor and 16 colours (the
`colors` values). The Python counterpart of Lua's `term` module (same
functions; Lua also has ComputerCraft's camelCase names).

    import term, colors
    term.clear()
    term.set_cursor_pos(1, 1)
    term.set_text_color(colors.yellow)
    term.write("Hello")              # no wrapping; term.write_wrapped wraps
    w, h = term.get_size()
    term.blit("abc", "0e4", "fff")   # text, text colours, backgrounds (blit digits)

Positions start at 1. Text written past the edge is cut off. A str is drawn
one character per cell (characters past 255 as "?"); bytes as they are.
term.redirect(t) sends every term call to `t` (any object with these
functions) and returns the previous target; term.native() is the console
itself, term.current() the target now. Python's print() still writes
straight to the console.
"""

import _console


class _Native:
    """The console itself, as a redirect target."""

    write = staticmethod(_console.write)
    blit = staticmethod(_console.blit)
    clear = staticmethod(_console.clear)
    clear_line = staticmethod(_console.clear_line)
    scroll = staticmethod(_console.scroll)
    get_cursor_pos = staticmethod(_console.get_cursor_pos)
    set_cursor_pos = staticmethod(_console.set_cursor_pos)
    get_cursor_blink = staticmethod(_console.get_cursor_blink)
    set_cursor_blink = staticmethod(_console.set_cursor_blink)
    get_size = staticmethod(_console.get_size)
    get_text_color = staticmethod(_console.get_text_color)
    set_text_color = staticmethod(_console.set_text_color)
    get_background_color = staticmethod(_console.get_background_color)
    set_background_color = staticmethod(_console.set_background_color)

    @staticmethod
    def is_color():
        return True

    @staticmethod
    def get_palette_color(colour):
        return _unpack(_console.get_palette_color(colour))

    @staticmethod
    def set_palette_color(colour, r, g=None, b=None):
        _console.set_palette_color(colour, r if g is None else _pack(r, g, b))


def _unpack(rgb):
    rgb = int(rgb)
    return ((rgb >> 16) & 0xFF) / 255, ((rgb >> 8) & 0xFF) / 255, (rgb & 0xFF) / 255


def _pack(r, g, b):
    ch = lambda v: int(max(0.0, min(1.0, v)) * 255)
    return (ch(r) << 16) | (ch(g) << 8) | ch(b)


_NAMES = ["write", "blit", "clear", "clear_line", "scroll", "get_cursor_pos", "set_cursor_pos",
          "get_cursor_blink", "set_cursor_blink", "get_size", "is_color", "get_text_color",
          "set_text_color", "get_background_color", "set_background_color", "get_palette_color",
          "set_palette_color"]

_native = _Native()
_current = _native


def write(text):
    """The text at the cursor, in the current colours (no wrapping)."""
    return _current.write(text)


def blit(text, text_colors, background_colors):
    """Text with a text and a background colour (blit digit) per character."""
    return _current.blit(text, text_colors, background_colors)


def clear():
    """The whole screen in the background colour; the cursor stays."""
    return _current.clear()


def clear_line():
    """The cursor's row in the background colour."""
    return _current.clear_line()


def scroll(n):
    """Move every row up n rows (down if negative)."""
    return _current.scroll(n)


def get_cursor_pos():
    return _current.get_cursor_pos()


def set_cursor_pos(x, y):
    return _current.set_cursor_pos(x, y)


def get_cursor_blink():
    return _current.get_cursor_blink()


def set_cursor_blink(on):
    return _current.set_cursor_blink(bool(on))


def get_size():
    return _current.get_size()


def is_color():
    return _current.is_color()


def get_text_color():
    return _current.get_text_color()


def set_text_color(colour):
    return _current.set_text_color(colour)


def get_background_color():
    return _current.get_background_color()


def set_background_color(colour):
    return _current.set_background_color(colour)


def get_palette_color(colour):
    """(r, g, b) in 0..1 that `colour` is shown as."""
    return _current.get_palette_color(colour)


def set_palette_color(colour, r, g=None, b=None):
    """set_palette_color(c, 0xRRGGBB) or set_palette_color(c, r, g, b) with 0..1 values."""
    return _current.set_palette_color(colour, r, g, b)


_DEFAULT = [0xF0F0F0, 0xF2B233, 0xE57FD8, 0x99B2F2, 0xDEDE6C, 0x7FCC19, 0xF2B2CC, 0x4C4C4C,
            0x999999, 0x4C99B2, 0xB266E5, 0x3366CC, 0x7F664C, 0x57A64E, 0xCC4C4C, 0x111111]


def native_palette_color(colour):
    """The default (r, g, b) of a colour."""
    if not isinstance(colour, (int, float)) or colour < 1 or colour >= 65536:
        raise ValueError("Colour out of range")
    return _unpack(_DEFAULT[int(colour).bit_length() - 1])


def redirect(target):
    """Send term calls to `target`; returns the previous target."""
    global _current
    missing = [n for n in _NAMES if not callable(getattr(target, n, None))]
    if missing:
        raise TypeError("redirect target is missing " + ", ".join(missing))
    old = _current
    _current = target
    return old


def current():
    return _current


def native():
    return _native


def write_wrapped(text):
    """Text wrapped at word boundaries ("\\n" starts a line), scrolling at the
    bottom; returns the number of lines started."""
    if _current is _native:
        return _console.write_wrapped(text)
    t = _current
    if isinstance(text, bytes):
        text = text.decode("latin-1")
    text = str(text)
    w, h = t.get_size()
    lines = 0

    def newline():
        nonlocal lines
        _, y = t.get_cursor_pos()
        if y + 1 <= h:
            t.set_cursor_pos(1, y + 1)
        else:
            t.set_cursor_pos(1, h)
            t.scroll(1)
        lines += 1

    i = 0
    while i < len(text):
        c = text[i]
        if c == "\n":
            newline()
            i += 1
        elif c in " \t":
            j = i
            while j < len(text) and text[j] in " \t":
                j += 1
            t.write(" " * (j - i))
            i = j
        else:
            j = i
            while j < len(text) and text[j] not in " \t\n":
                j += 1
            word = text[i:j]
            x, _ = t.get_cursor_pos()
            if len(word) > w - x + 1 and x > 1:
                newline()
                x = 1
            while len(word) > w - x + 1:
                room = max(1, w - x + 1)
                t.write(word[:room])
                newline()
                word = word[room:]
                x = 1
            t.write(word)
            i = j
    return lines
