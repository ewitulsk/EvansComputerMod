"""Lidar outline viewer: a top-down map of what a lidar sees, redrawn after
every scan. Needs a Wired Sensor Module with a lidar wired to it.

    python lidar_view.py            # first lidar
    python lidar_view.py lidar_2    # a named one

The map is centred on the lidar. '#' is a hit, 'O' the lidar, '@' the
computer. The top of the screen is the side away from someone looking at
the screen (set FACING_VIEWER = False to put the computer's forward at the
top instead, e.g. on a car). Each column is half a block and each row one
block, so the map isn't squashed.
"""

import sys
import sensors
import terminal

RANGE = 16            # blocks
AZ_STEPS = 360        # rays per sweep
ELEVATION = 0         # degrees, the sweep's tilt
FACING_VIEWER = True
COL_BLOCKS = 0.5
ROW_BLOCKS = 1.0


def main():
    hub = sensors.find()
    if hub is None:
        print("No Wired Sensor Module in this computer's bays.")
        return
    argv = getattr(sys, "argv", None) or []
    name = argv[1] if len(argv) > 1 else None
    waited = 0
    while not hub.names():
        if waited == 0:
            print("Waiting for a lidar on the wires...")
        terminal.sleep(0.5)
        waited += 1
        if waited > 20:
            print("No lidar found. Wire one to the module's connector.")
            return
    if name is None:
        name = hub.names()[0]
    lidar = hub.lidar(name)
    lidar.configure(az_min=-180, az_max=180, az_steps=AZ_STEPS, el_min=ELEVATION, el_max=ELEVATION,
                    rows=1, range=RANGE)
    lidar.start()
    mount = lidar.mount()
    sx, sy = mount["x"], mount["y"]

    width = terminal.get_width() - 1
    height = terminal.get_height()
    rows = height - 2
    cx, cy = width // 2, rows // 2

    def cell(x, y):
        """Computer-frame point -> (column, row) on the map, centred on the lidar."""
        dx, dy = x - sx, y - sy
        if FACING_VIEWER:
            # The viewer faces the screen, i.e. looks along -x: their right is +y.
            col, row = cx + dy / COL_BLOCKS, cy + dx / ROW_BLOCKS
        else:
            col, row = cx - dy / COL_BLOCKS, cy - dx / ROW_BLOCKS
        return int(round(col)), int(round(row))

    terminal.clear()
    seq = 0
    while True:
        scan = lidar.wait(after=seq, timeout=5)
        if scan is None:
            continue
        seq = scan.seq
        grid = [[" "] * width for _ in range(rows)]
        for x, y, _z in lidar.points():
            col, row = cell(x, y)
            if 0 <= col < width and 0 <= row < rows:
                grid[row][col] = "#"
        col, row = cell(0.0, 0.0)
        if 0 <= col < width and 0 <= row < rows:
            grid[row][col] = "@"
        grid[cy][cx] = "O"

        header = "LIDAR %s  scan %d  hits %d/%d  range %d  ticks %d-%d" % (
            name, scan.seq, scan.hits, scan.columns * scan.rows, RANGE, scan.start_tick, scan.end_tick)
        terminal.set_cursor(0, 0)
        terminal.write(header[:width].ljust(width))
        for r in range(rows):
            terminal.set_cursor(0, r + 1)
            terminal.write("".join(grid[r]))
        terminal.set_cursor(0, height - 1)
        terminal.write("O lidar  @ computer  # hit   (Ctrl+C to quit)"[:width].ljust(width))


main()
