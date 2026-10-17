"""Redstone by side name: "top", "bottom", "front", "back", "left",
"right" (left and right as you face the computer's screen). The Python
counterpart of Lua's `redstone` (`rs`) module (same functions).

    import redstone
    redstone.set_output("top", True)
    redstone.get_input("left")              # True while a lever on the left is on
    redstone.set_analog_output("back", 7)
    redstone.get_sides()                    # ["top", "bottom", "left", "right", "front", "back"]

Bundled cables need a mod that adds them: here bundled input is 0 and
bundled output is ignored.
"""

import _console


def get_sides():
    return ["top", "bottom", "left", "right", "front", "back"]


def get_input(side):
    return _console.rs_get_input(side) > 0


def get_output(side):
    return _console.rs_get_output(side) > 0


def set_output(side, on):
    _console.rs_set_output(side, 15 if on else 0)


def get_analog_input(side):
    return _console.rs_get_input(side)


def get_analog_output(side):
    return _console.rs_get_output(side)


def set_analog_output(side, power):
    if not isinstance(power, (int, float)) or power < 0 or power > 15:
        raise ValueError("Expected number in range 0-15")
    _console.rs_set_output(side, int(power))


def get_bundled_input(side):
    _console.rs_get_input(side)
    return 0


def get_bundled_output(side):
    _console.rs_get_output(side)
    return 0


def set_bundled_output(side, colours):
    _console.rs_get_output(side)


def test_bundled_input(side, colour):
    return get_bundled_input(side) & int(colour) == int(colour)
