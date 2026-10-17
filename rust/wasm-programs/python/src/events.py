"""The program's event queue: keys and typed characters, timers, in-game
alarms, peripheral events and events the program queues itself, oldest
first. The Python counterpart of Lua's `events` module (same functions);
peripheral.pull_event reads the same queue.

    import events, keys
    t = events.start_timer(2)
    while True:
        ev = events.pull()
        if ev[0] == "key" and ev[1] == keys.enter: print("enter")
        elif ev[0] == "char": print("typed", ev[1])
        elif ev[0] == "timer" and ev[1] == t: break

Events: ("key", code, held) / ("key_up", code) / ("char", text) /
("timer", id) / ("alarm", id) / peripheral events (see peripheral) / your own
queue(name, *args). pull(filter) discards events with other names while it
waits ("terminate" always comes through). timeout in seconds: None waits
forever, 0 only looks; None on timeout.
"""

import _console


def pull(filter=None, timeout=None):
    """The next event as (name, *args), or None on timeout."""
    return _console.pull(filter, timeout)


def queue(name, *args):
    """Add an event to the queue."""
    _console.queue(str(name), list(args))


def start_timer(seconds):
    """A ("timer", id) event after `seconds` (in whole game ticks of 0.05 s); returns its id."""
    return _console.start_timer(seconds)


def cancel_timer(id):
    _console.cancel_timer(id)


def set_alarm(hour):
    """An ("alarm", id) event when the world's clock next reaches `hour` (0 <= hour < 24)."""
    return _console.set_alarm(hour)


def cancel_alarm(id):
    _console.cancel_alarm(id)


def sleep(seconds=0):
    """Wait `seconds` (at least one tick); events that arrive meanwhile are discarded."""
    _console.sleep(seconds)
