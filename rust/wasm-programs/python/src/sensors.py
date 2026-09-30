"""Wired sensors: lidars wired to a Wired Sensor Module in one of the bays.

    import sensors

    hub = sensors.find()                    # the first Wired Sensor Module
    print(hub.names())                      # ['lidar_1', 'lidar_2']
    front = hub.lidar("lidar_1")
    print(front.mount())                    # {'x': 1.5, 'y': 0.0, 'z': -0.4, 'yaw': 0.0, ...}
    front.configure(az_min=-60, az_max=60, az_steps=121, el_min=-10, el_max=10, rows=3, range=24)

    scan = front.scan()                     # take one scan and wait for it
    print(scan.seq, scan.range_at(1, 60))   # middle row, straight ahead
    for x, y, z in front.points():          # hits in the computer's frame
        ...

    front.start()                           # scan continuously
    while True:
        scan = front.wait(after=scan.seq)   # the next finished scan

Frames. A lidar sweeps around the structure's up axis: azimuth 0 is the
sensor's forward (out of the wall it is on, or where you looked when placing
it on a floor or ceiling), positive azimuth turns left (counter-clockwise
seen from above), positive elevation is up. Points and mounts use the
computer's frame: x = the way its screen faces, y = left, z = up, origin at
the computer's centre, in blocks. Mounts never change while the vehicle
moves, since the computer and its sensors move together.

Ranges are distances in blocks; float('inf') means nothing within range.
Rows go from el_min to el_max, columns from az_min to az_max (a full 360
degree sweep doesn't repeat its first column).
"""

import peripheral
import _peripheral

INF = float("inf")


def unpack_f32(data):
    """Little-endian float32 bytes -> list of floats."""
    return _peripheral.unpack_f32(data)


class Scan:
    """One finished lidar scan."""

    def __init__(self, info):
        self.seq = info["seq"]
        self.start_tick = info["start_tick"]
        self.end_tick = info["end_tick"]
        self.rows = info["rows"]
        self.columns = info["columns"]
        self.config = info["config"]
        self.hits = info["hits"]
        self.ranges = unpack_f32(info["ranges"])

    def range_at(self, row, column):
        return self.ranges[row * self.columns + column]

    def row(self, row):
        c = self.columns
        return self.ranges[row * c:(row + 1) * c]

    def azimuth(self, column):
        """Azimuth of a column, degrees."""
        cfg = self.config
        lo, hi, n = cfg["az_min"], cfg["az_max"], self.columns
        if hi - lo >= 360 - 1e-9:
            return lo + column * (hi - lo) / n
        return lo if n == 1 else lo + column * (hi - lo) / (n - 1)

    def elevation(self, row):
        """Elevation of a row, degrees."""
        cfg = self.config
        lo, hi = cfg["el_min"], cfg["el_max"]
        return lo if self.rows == 1 else lo + row * (hi - lo) / (self.rows - 1)

    def nearest(self):
        """(range, row, column) of the closest hit, or None."""
        best = None
        for i, r in enumerate(self.ranges):
            if r != INF and (best is None or r < best[0]):
                best = (r, i // self.columns, i % self.columns)
        return best

    def __repr__(self):
        return "<lidar scan %d: %dx%d, %d hits>" % (self.seq, self.rows, self.columns, self.hits)


class Lidar:
    """One lidar on a Wired Sensor Module."""

    def __init__(self, hub, name):
        self.hub = hub
        self.name = name

    def _call(self, method, *args):
        return self.hub.peripheral.call(method, self.name, *args)

    def mount(self):
        """Where the sensor sits: {x, y, z, yaw, mount, block, facing} (computer frame)."""
        return self._call("mount")

    def configure(self, **settings):
        """Set any of az_min, az_max, az_steps, el_min, el_max, rows, range; returns the full setting."""
        return self._call("configure", settings)

    def config(self):
        return self._call("get_config")

    def start(self):
        """Scan continuously (survives reloads and moves)."""
        self._call("start")

    def stop(self):
        self._call("stop")

    def seq(self):
        """Sequence number of the latest finished scan (0 = none yet)."""
        return self._call("get_seq")

    def latest(self):
        """The latest finished Scan, or None."""
        info = self._call("get_scan")
        return None if info is None else Scan(info)

    def wait(self, after=0, timeout=None):
        """Wait for a scan with seq > `after` and return it (None on timeout, in seconds)."""
        waited = 0.0
        while self.seq() <= after:
            if timeout is not None and waited >= timeout:
                return None
            peripheral.pull_event("lidar_scan", timeout=0.25)
            waited += 0.25
        return self.latest()

    def scan(self, timeout=10):
        """Take one scan and return it (None on timeout)."""
        target = self._call("scan")
        return self.wait(after=target - 1, timeout=timeout)

    def points(self):
        """Hits of the latest scan as (x, y, z) tuples in the computer frame."""
        info = self._call("get_points")
        if info is None:
            return []
        f = unpack_f32(info["points"])
        return [(f[i], f[i + 1], f[i + 2]) for i in range(0, len(f), 3)]

    def __repr__(self):
        return "<lidar '%s' on %s>" % (self.name, self.hub.peripheral.name)


class Hub:
    """A Wired Sensor Module."""

    def __init__(self, p):
        self.peripheral = p

    def names(self):
        return self.peripheral.names()

    def list(self):
        """[{name, type, mount, running}, ...]"""
        return self.peripheral.list()

    def lidar(self, name):
        if name not in self.names():
            raise peripheral.PeripheralError("no sensor named '%s' (have %s)" % (name, self.names()))
        return Lidar(self, name)

    def lidars(self):
        return [Lidar(self, s["name"]) for s in self.list() if s["type"] == "lidar"]

    def __repr__(self):
        return "<wired sensors on %s: %s>" % (self.peripheral.name, ", ".join(self.names()))


def find():
    """The first Wired Sensor Module, or None."""
    p = peripheral.find("wired_sensors")
    return None if p is None else Hub(p)


def find_all():
    return [Hub(p) for p in peripheral.find_all("wired_sensors")]
