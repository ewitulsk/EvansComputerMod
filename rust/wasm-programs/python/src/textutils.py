"""Values as text and back, and text on the screen. The Python counterpart
of Lua's `textutils` module (same functions; Lua also has ComputerCraft's
camelCase names).

    import textutils
    s = textutils.serialize({"name": "x", "list": [1, 2]})   # a Lua literal, CC's layout
    textutils.unserialize(s)                                 # back to dicts and lists
    textutils.serialize_json({"a": [1, 2]})                  # '{"a":[1,2]}'
    textutils.unserialize_json('{"a":[1,2]}')
    textutils.format_time(13.5)                              # "1:30 PM" (True: "13:30")
    textutils.url_encode("a b&c")                            # "a+b%26c"
    textutils.tabulate(["Name", "Size"], ["a", "1"])         # columns on the term

Lists are Lua lists (keys 1..n), dicts other tables. Dict keys come out sorted.
"""

import _console


def serialize(value, compact=False):
    return _console.serialize(value, bool(compact))


def unserialize(text):
    """The value of a Lua literal, or None if it isn't one."""
    try:
        return _console.unserialize(text)
    except ValueError:
        return None


def serialize_json(value, unicode=False):
    return _console.serialize_json(value, bool(unicode))


def unserialize_json(text):
    """(value) of a JSON text; raises ValueError if it isn't JSON."""
    return _console.unserialize_json(text)


def format_time(t, twenty_four=False):
    return _console.format_time(t, bool(twenty_four))


def url_encode(text):
    return _console.url_encode(text)


def slow_write(text, rate=20):
    """Write `text` a character at a time (`rate` characters a second)."""
    import events
    for ch in str(text):
        events.sleep(1 / rate)
        _console.write_wrapped(ch)


def slow_print(text, rate=20):
    slow_write(text, rate)
    _console.write_wrapped("\n")


def paged_print(text, free_lines=0):
    """Print `text` on the term; when the screen fills up, wait for a key.
    Returns the number of lines printed."""
    import events
    _, h = _console.get_size()
    shown, total = free_lines, 0
    for line in str(text).split("\n"):
        if shown >= h - 1:
            _console.write_wrapped("Press any key to continue")
            events.pull("key")
            _console.clear_line()
            _, y = _console.get_cursor_pos()
            _console.set_cursor_pos(1, y)
            shown = 0
        n = _console.write_wrapped(line + "\n")
        shown += n
        total += n
    return total


def _tabulate(paged, rows):
    w, _ = _console.get_size()
    widest = max([len(str(c)) + 1 for r in rows if isinstance(r, (list, tuple)) for c in r] or [1])
    cols = max(1, w // widest)
    width = w // cols
    lines = 0
    for row in rows:
        if isinstance(row, (int, float)):
            _console.set_text_color(row)
            continue
        line = ""
        for i, cell in enumerate(row, 1):
            s = str(cell)
            line += s
            if i % cols == 0 or i == len(row):
                lines += paged_print(line) if paged else _console.write_wrapped(line + "\n")
                line = ""
            else:
                line += " " * (width - len(s))
    return lines


def tabulate(*rows):
    """Rows (lists of cells) in columns as wide as the widest cell; a colour
    between rows sets the text colour from there on."""
    return _tabulate(False, rows)


def paged_tabulate(*rows):
    return _tabulate(True, rows)
