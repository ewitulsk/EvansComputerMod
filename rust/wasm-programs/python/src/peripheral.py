"""Peripherals: blocks next to the computer and modules in its bays.

    import peripheral

    peripheral.names()                 # ['left', 'left_bay_1']
    link = peripheral.find("redstone_link")
    link.set_channel(0, "minecraft:red_dye", "minecraft:red_dye", "tx")
    link.set_output(0, 15)

    event = peripheral.pull_event("redstone_link", timeout=5)
    if event:
        name, attachment, channel, power, old = event

Attachment names: sides "front", "back", "left", "right", "top", "bottom"
(left/right as for redstone sides) and bay slots "left_bay_1", "left_bay_2",
"right_bay_1", "right_bay_2".

Events are tuples (event, attachment, *args). Built-in events:
    ("peripheral", name, type)       something was attached
    ("peripheral_detach", name)      it went away
"""

import _peripheral


class PeripheralError(Exception):
    """Raised when a peripheral rejects a call (bad argument, no such peripheral, ...)."""


def attached():
    """[(name, type), ...] for everything attached to this computer."""
    return _peripheral.list()


def names():
    """Names of all attached peripherals."""
    return [n for n, _ in _peripheral.list()]


def is_present(name):
    return any(n == name for n, _ in _peripheral.list())


def get_type(name):
    """Type of the peripheral attached as `name`, or None."""
    for n, t in _peripheral.list():
        if n == name:
            return t
    return None


def get_methods(name):
    """Method names of the peripheral attached as `name`, or None."""
    ok, result = _peripheral.methods(name)
    return result[1] if ok else None


def call(name, method, *args):
    """Call `method` on the peripheral attached as `name`."""
    ok, result = _peripheral.call(name, method, list(args))
    if not ok:
        raise PeripheralError(result)
    return result


class Peripheral:
    """A wrapped peripheral: its methods are attributes, e.g. link.set_output(0, 15)."""

    def __init__(self, name, type, methods):
        self.name = name
        self.type = type
        self._methods = list(methods)

    def methods(self):
        return list(self._methods)

    def call(self, method, *args):
        return call(self.name, method, *args)

    def __getattr__(self, attr):
        if attr.startswith("_") or attr not in self._methods:
            raise AttributeError("%s peripheral '%s' has no method '%s'" % (self.type, self.name, attr))
        name = self.name

        def method(*args):
            return call(name, attr, *args)

        method.__name__ = attr
        return method

    def __dir__(self):
        return ["name", "type", "methods", "call"] + self._methods

    def __eq__(self, other):
        return isinstance(other, Peripheral) and other.name == self.name and other.type == self.type

    def __hash__(self):
        return hash((self.name, self.type))

    def __repr__(self):
        return "<peripheral %s '%s'>" % (self.type, self.name)


def wrap(name):
    """Peripheral attached as `name`, or None."""
    ok, result = _peripheral.methods(name)
    if not ok:
        return None
    return Peripheral(name, result[0], result[1])


def find_all(type):
    """Every attached peripheral of `type`, in attachment order."""
    out = []
    for n, t in _peripheral.list():
        if t == type:
            p = wrap(n)
            if p is not None:
                out.append(p)
    return out


def find(type):
    """First attached peripheral of `type`, or None."""
    for n, t in _peripheral.list():
        if t == type:
            p = wrap(n)
            if p is not None:
                return p
    return None


def pull_event(filter=None, timeout=None):
    """Wait for the next event (named `filter`, if given) and return it as
    (event, attachment, *args). Other events are discarded while waiting.
    `timeout` in seconds; None waits forever, 0 only checks. Returns None
    on timeout."""
    if timeout is None:
        ms = -1
    else:
        ms = min(max(0, int(timeout * 1000)), 2147483647)
    return _peripheral.wait_event(filter, ms)
