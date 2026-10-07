"""Wireless Xbox Controller (peripheral type "xbox_controller").

A connected controller is attached as controller_1 .. controller_4, its player
number. Programs see Xbox buttons and axes only: which keys drive them is set
on the controller item (sneak + right-click it).

    import controller

    pad = controller.find()             # lowest player number, or None
    s = pad.state()                     # {'a': False, ..., 'lx': 0.0, ..., 'player': 1}
    if pad.is_down("a"):
        print("jump")
    x, y = pad.axis("lx"), pad.axis("ly")   # -1..1, right and up positive

    while True:
        ev = controller.pull_event()    # ('button', 'controller_1', 'a', True)
        print(ev)                       # ('axis', 'controller_1', 'lx', -1.0)

Buttons: a b x y lb rb back start guide ls rs dpad_up dpad_down dpad_left dpad_right
Axes: lx ly rx ry (-1..1), lt rt (0..1)
"""

import peripheral

TYPE = "xbox_controller"
BUTTONS = ["a", "b", "x", "y", "lb", "rb", "back", "start", "guide", "ls", "rs",
           "dpad_up", "dpad_down", "dpad_left", "dpad_right"]
AXES = ["lx", "ly", "rx", "ry", "lt", "rt"]


class Controller:
    """One connected controller."""

    def __init__(self, name):
        self.name = name

    @property
    def player(self):
        """Player number, 1-4."""
        return int(self.name.rsplit("_", 1)[1])

    def connected(self):
        return peripheral.get_type(self.name) == TYPE

    def state(self):
        """Every button (bool) and axis (float) at once, plus 'player'."""
        return peripheral.call(self.name, "get_state")

    def is_down(self, button):
        return peripheral.call(self.name, "is_down", button)

    def axis(self, name):
        return peripheral.call(self.name, "get_axis", name)

    def buttons(self):
        """Names of the buttons held now."""
        return peripheral.call(self.name, "get_buttons")

    def raw(self):
        """[button bit mask (bit i = BUTTONS[i]), lx, ly, rx, ry (-127..127), lt, rt (0..255)]"""
        return peripheral.call(self.name, "get_raw")

    def __repr__(self):
        return "Controller(%r)" % self.name


def all():
    """Every connected controller, by player number."""
    pads = [Controller(name) for name, t in peripheral.attached() if t == TYPE]
    pads.sort(key=lambda p: p.player)
    return pads


def find(player=None):
    """The controller of `player` (1-4), or the lowest-numbered one; None if none is connected."""
    for pad in all():
        if player is None or pad.player == player:
            return pad
    return None


def pull_event(timeout=None):
    """Next controller event as ('button', name, button, pressed) or ('axis', name, axis, value);
    None after `timeout` seconds. Other peripheral events are skipped."""
    import time
    deadline = None if timeout is None else time.time() + timeout
    while True:
        left = None if deadline is None else max(0, deadline - time.time())
        ev = peripheral.pull_event(None, left)
        if ev is None:
            return None
        if ev[0] == "controller_button":
            return ("button",) + tuple(ev[1:])
        if ev[0] == "controller_axis":
            return ("axis",) + tuple(ev[1:])
