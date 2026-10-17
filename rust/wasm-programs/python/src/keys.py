"""Key codes, as ("key", code, held) and ("key_up", code) events give them
(the GLFW codes ComputerCraft uses). The Python counterpart of Lua's `keys`.

    import events, keys
    ev = events.pull("key")
    if ev[1] == keys.enter: ...
    keys.get_name(ev[1])       # "enter"

Names: a-z, zero-nine, f1-f25, numPad0-numPad9, space, enter, tab, backspace,
delete, insert, up, down, left, right, home, end (keys.get("end")), pageUp,
pageDown, leftShift, rightShift, leftCtrl, rightCtrl, leftAlt, rightAlt, ...
"""

import _console

_by_name = _console.key_names()
_by_code = {v: k for k, v in _by_name.items()}
globals().update({k: v for k, v in _by_name.items() if k != "end"})


def get(name):
    """The code of key `name` (for names that are Python keywords, like "end")."""
    return _by_name.get(name)


def get_name(code):
    """The name of a key code, or None."""
    return _by_code.get(code)
