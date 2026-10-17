"""The 16 colours, as numbers with one bit each (white = 1 ... black =
32768), so a set of colours is their sum. The Python counterpart of Lua's
`colors` module (same names; Lua adds ComputerCraft's camelCase names).

    import term, colors
    term.set_text_color(colors.red)
    colors.combine(colors.red, colors.blue)   # a set
    colors.test(s, colors.red)                # True
    colors.to_blit(colors.red)                # "e"
    colors.pack_rgb(1, 0.5, 0)                # 0xFF7F00
"""

white, orange, magenta, lightBlue = 0x1, 0x2, 0x4, 0x8
yellow, lime, pink, gray = 0x10, 0x20, 0x40, 0x80
lightGray, cyan, purple, blue = 0x100, 0x200, 0x400, 0x800
brown, green, red, black = 0x1000, 0x2000, 0x4000, 0x8000
light_blue, light_gray = lightBlue, lightGray


def combine(*colours):
    """The set of all the colours (and sets) given."""
    r = 0
    for c in colours:
        r |= int(c)
    return r


def subtract(set, *colours):
    """`set` without the colours (and sets) given."""
    r = int(set)
    for c in colours:
        r &= ~int(c)
    return r


def test(set, colour):
    """Whether `set` holds all of `colour`."""
    return int(set) & int(colour) == int(colour)


def pack_rgb(r, g, b):
    """0xRRGGBB from red, green and blue in 0..1."""
    ch = lambda v: int(max(0.0, min(1.0, v)) * 255)
    return (ch(r) << 16) | (ch(g) << 8) | ch(b)


def unpack_rgb(rgb):
    """(r, g, b) in 0..1 from 0xRRGGBB."""
    rgb = int(rgb)
    return ((rgb >> 16) & 0xFF) / 255, ((rgb >> 8) & 0xFF) / 255, (rgb & 0xFF) / 255


def to_blit(colour):
    """The blit digit ("0"-"f") of a colour."""
    if not isinstance(colour, (int, float)) or colour < 1 or colour >= 65536:
        raise ValueError("Colour out of range")
    return "0123456789abcdef"[int(colour).bit_length() - 1]


def from_blit(digit):
    """The colour of a blit digit, or None."""
    if not isinstance(digit, str) or len(digit) != 1 or digit.lower() not in "0123456789abcdef":
        return None
    return 1 << int(digit, 16)
