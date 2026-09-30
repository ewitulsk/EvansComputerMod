# EvansComputerMod

A Minecraft mod that adds in-world computer terminals powered by WebAssembly. Each terminal runs a Rust kernel compiled to WASM; programs (shell utilities, networking tools, an embedded Python interpreter based on RustPython, `ssh`, `httpd`, …) are separate WASI binaries the kernel launches as child processes.

**Minecraft:** 26.1 (primary) and 1.21.1 (backport) | **Mod Loader:** NeoForge 26.1.0.1-beta / 21.1.77+

Dual-version support is handled by [Stonecutter](https://stonecutter.kikugie.dev) from a single `src/` tree. One `./gradlew chiseledBuild` (or `./copy-jar.sh`) produces both `evanscomputermod-mc26.1-*.jar` and `evanscomputermod-mc1.21.1-*.jar`. Building MC 26.1 needs Java 25; MC 1.21.1 needs Java 21 — the NeoForge toolchain selects the right JDK automatically as long as both are installed.

## Architecture

```
  WASI programs (ls, ping, curl, httpd, ssh, python, ...)   one thread each
        |  blocking socket / stdio calls
  Java host: ComputerInstance worker thread ---- NetworkHub (cable segments) ---- TAP bridge
        |  exports: main, on_input, on_interrupt, on_tick, handle_sock_ipc, ...
  Rust kernel (terminal_os.wasm)  -- event-driven, never blocks
        |- shell + job control
        |- Net: the only owner of the NICs
        |     |- ecm-net   : sans-IO IPv4 stack (ARP/IP/ICMP/UDP/TCP/DNS), sockets for programs
        |     '- ecm-bridge: L2 switch (VLANs, RSTP, LACP, LLDP) when `switch` is running
        '- console (VTE -> text framebuffer), gfx planes
```

The kernel is **event-driven**: every export does a bounded amount of work and returns. The host's worker thread owns all waiting — it calls `on_tick(now)` when the kernel's returned deadline passes or when an event arrives (keystrokes, network frames, child output, socket requests). No host function blocks or calls back into the kernel. Child processes block freely on their own threads; their socket calls are proxied to the kernel, which answers immediately or says "pending" and is retried.

Design and rationale: [`docs/refactor/ARCHITECTURE.md`](docs/refactor/ARCHITECTURE.md). Review that motivated it: [`docs/kernel-review-2026-09.md`](docs/kernel-review-2026-09.md).

### Routing and Tech Villages

Terminals can run an IPv4 router with DHCP, NAT and BGP using `router on` and
`router`. The 1.21.1 build also generates ten Tech Villages connected by a fiber
ring, with twenty server-owned ISP/router computers that keep running outside
loaded chunks. Player terminals can use an Always-On bay module. Windows servers
have a socket-based internet gateway without a TAP driver.

See **[the router guide](docs/ROUTER.md)** for CLI contexts, complete LAN/WAN and
BGP examples, port forwarding, DHCP leases, routing policies, customer ports,
player AS peering, village commands, persistence and implementation details.

### Kernel ↔ host ABI

The complete list of kernel imports and exports, with signatures and semantics, is [`abi/host-abi.toml`](abi/host-abi.toml). It is enforced three ways: `scripts/check-abi.py` diffs a built kernel against it, the Java host refuses to load a kernel whose imports it doesn't implement exactly (`ComputerInstance.checkKernelImports`), and the simulator links just as strictly.

Shared memory has no fixed addresses: the input, interrupt, socket-IPC, framebuffer and graphics buffers are kernel statics, and the host reads their addresses and sizes from the `abi_layout` export after instantiation.

## Getting Started

1. Place a **Terminal** block (found in the Redstone and Functional Blocks creative tabs)
2. Right-click to open the terminal
3. Type `help` to see available commands

## Blocks

| Block | Description |
|-------|-------------|
| **Terminal** | The computer itself. Has 5 usable network faces; the screen has no NIC. Right-click to open. Holds up to four modules in two side bays (see [Modules and Peripherals](#modules-and-peripherals)). |
| **Network Cable** | Connects computers and interfaces together. Visually connects to adjacent cables, terminals, interfaces, and the gateway. |
| **Network Interface** | Expansion block — attach to a terminal (or chain to another interface block) to add more network interfaces. Each free face becomes a new ethN interface. |
| **Screen** | In-world display block. Place adjacent to a Terminal and its output renders on the face. Multiple adjacent Screens sharing the same facing form a single rectangle-shaped cluster whose resolution scales with the tile count (128×72 per tile). |
| **Redstone Link Interface** | *(Create installed)* Place next to a Terminal to give it a `redstone_link` peripheral: 64 programmable Create Redstone Link channels. Keeps its channel setup when broken. |
| **Internet Gateway** | Unbreakable block at (0,0,0) providing real internet access via TAP bridge. Auto-generated with cable column to surface on first server start. |

### Network Interface Block

The **Network Interface** block expands a terminal's networking capability beyond its built-in interfaces.

**Terminal screen face:** The front face of the terminal (the screen side, determined by placement direction) has **no interface** and does not accept cables or Interface blocks. This leaves **5 usable faces** on the terminal.

**How it works:**
- A bare terminal has **5 interfaces** (one per non-screen face)
- Place an Interface block on a terminal face → that face's built-in interface is replaced by the Interface block's 5 free faces. Result: **4 + 5 = 9 interfaces**
- Chain Interface blocks → each additional block in the chain adds 5 more
- Interfaces are numbered sequentially (eth0, eth1, ...) starting from the terminal's non-screen faces, then the Interface block free faces

```
                    ┌─────────────────┐
                    │  Interface Block │ ← 5 free faces = eth5-eth9
                    └────────┬────────┘
                             │ (replaces terminal's UP face)
┌────────────────────────────┴────────────────────────────┐
│                      Terminal Block                      │
│  Screen face (FRONT) = no interface                      │
│  eth0=DOWN  (UP occupied)  eth1=BACK  eth2=LEFT eth3=RIGHT │
└──────────────────────────────────────────────────────────┘
```

**Built-in interfaces (5 per terminal, screen face excluded):**

The terminal's FACING direction (set when placed) is the screen — it gets no interface. The remaining 5 faces each get one interface, numbered sequentially. The exact mapping depends on which direction the terminal faces and whether any faces have Interface blocks.

**Example** (terminal facing NORTH, no Interface blocks):

| Interface | Face |
|-----------|------|
| eth0 | DOWN |
| eth1 | UP |
| eth2 | SOUTH (back) |
| eth3 | WEST (left) |
| eth4 | EAST (right) |

NORTH (screen) is excluded.

### Link State Visual Feedback

When you set an interface to **down** (`ifconfig eth0 down` or `ip link set eth0 down`), the cable connected to that face of the terminal visually **disconnects**. Setting it back to **up** **reconnects** it. This provides real-time visual feedback about which interfaces are active.

### Screen Block

An in-world display block. Place a Screen next to a Terminal (on any
non-screen face) to attach it to that computer.

- **Cluster resolution scales with tile count.** Per-tile default is **128×72
  pixels** (graphics) or equivalently **32×18 cells** (text). A 2×2 cluster is
  256×144; a 4×2 cluster is 512×144.
- **Rectangle-only.** Adjacent screens that share the same facing direction
  merge into one logical display, but only if the total footprint is a
  perfect rectangle. L-shapes and other non-rectangular arrangements stay
  inactive until fixed.
- **Anchor.** The screen at the top-left corner (viewed from outside the
  display) is the anchor; its BlockEntityRenderer draws a single textured
  quad that spans the entire cluster in world space.
- **Text + graphics + overlay.** All three display modes are supported. Text
  and overlay text are rasterized into the graphics buffer by the Rust OS
  using a built-in 5×7 bitmap font, so the client only has to render one
  textured quad per cluster.
- **Usage.** From the Rust OS, call the `screen` module (`screen::is_attached`,
  `screen::init`, `screen::set_pixel`, `screen::fill_rect`, `screen::draw_text`,
  `screen::sync`). Legacy programs that don't know about the screen are
  unaffected — the host exposes `screen_is_attached` so programs can
  gracefully no-op when no cluster is present. A Python `screen` module
  binding is still TODO.

Try it out: `gfxtest screen`.

## Modules and Peripherals

Programs talk to hardware through **peripherals**: blocks next to the computer, and **modules** installed inside it. Modules keep a computer to a single block, which matters on a Create Aeronautics vehicle.

### Module bays

1. Right-click a Terminal with a **Module Expansion Card**. It opens a two-slot bay on the Terminal's left side (the side you clicked, if that's a free bay side). A second card opens the right bay, for four slots in all. The bays are recessed into the block, so it stays exactly one block.
2. Right-click the Terminal with a **module** to install it. It goes into the slot you aim at (upper or lower half of a bay), otherwise the first free slot. The module shows up in the slot.
3. **Sneak-right-click a slot with an empty hand** to take a module out. Sneak-right-click an empty bay to take its card back.

Breaking the Terminal keeps its cards and modules, with each module's settings, on the dropped item. Placing it again puts everything back. Slots are named `left_bay_1` (upper), `left_bay_2` (lower), `right_bay_1` and `right_bay_2`. Left and right are the Terminal's own sides, as for redstone.

| Item | Recipe |
|------|--------|
| **Module Expansion Card** | `iron nugget, redstone, iron nugget` / `copper ingot, gold ingot, copper ingot` |
| **Redstone Link Module** *(Create)* | `_, transmitter, _` / `brass sheet, electron tube, brass sheet` / `_, redstone, _` |
| **Redstone Link Interface** *(Create)* | `_, transmitter, _` / `electron tube, brass casing, electron tube` |

### Peripheral names

| Where | Name |
|-------|------|
| Block on a side | `front`, `back`, `left`, `right`, `top`, `bottom` |
| Module in a bay | `left_bay_1`, `left_bay_2`, `right_bay_1`, `right_bay_2` |

Run `peripherals` in the shell to list what's attached, or `peripherals <name>` for a peripheral's methods.

### Redstone Link (`redstone_link`)

Needs [Create](https://modrinth.com/mod/create) (6.0.10+). The Redstone Link Module and the Redstone Link Interface provide the same peripheral: **64 channels** that talk to Create's Redstone Links, Linked Controllers and other computers.

- **Frequency:** each channel has one, as the two items of a Create link's frequency slots, given by item id (`"minecraft:red_dye"`). Like Create, a frequency only compares the item and, for dyed items, the colour: pass `{"item": id, "color": 0xRRGGBB}` for those. `None` is an empty slot.
- **Mode:** each channel is `"tx"` (transmit its output strength), `"rx"` (receive the strongest transmitter in range) or `"off"`.
- **Range:** Create's `linkRange` setting (256 blocks by default). On a Sable / Create Aeronautics structure, range is measured in world space, and it keeps working while the structure moves.

| Method | Description |
|--------|-------------|
| `get_channel_count()` | `64` |
| `set_channel(ch, first, second, mode)` | Configure channel `ch` (0-63) |
| `set_frequency(ch, first, second)` / `set_mode(ch, mode)` | Change one part of a channel |
| `set_output(ch, power)` / `get_output(ch)` | Strength a tx channel transmits (0-15) |
| `set_outputs({ch: power, ...})` | Several outputs in one call |
| `get_input(ch)` / `get_inputs()` | Strength an rx channel receives / all 64 |
| `get_channel(ch)` / `get_channels()` | A channel's setup and strengths / every configured channel |
| `clear(ch)` / `clear_all()` | Turn channels off and forget their frequencies |

**Event:** `("redstone_link", name, channel, power, old)` when an rx channel's received strength changes (at most once per channel per tick).

```python
import peripheral

link = peripheral.find("redstone_link")
link.set_channel(0, "minecraft:red_dye", "minecraft:red_dye", "tx")   # a door
link.set_channel(1, "minecraft:lever", "minecraft:lever", "rx")       # a lever on a Create transmitter

while True:
    event = peripheral.pull_event("redstone_link")
    _, _, channel, power, old = event
    if channel == 1:
        link.set_output(0, 15 if power > 0 else 0)
```

The channel setup is saved on the module item (or the interface block), so it survives restarts, moving the module and breaking the block.

### Wired sensors and lidar (`wired_sensors`, 1.21.1)

A **Lidar Sensor** is a small puck you mount on a floor, wall or ceiling (under a car, on its front bumper). A **Wired Sensor Module** in a bay reads up to 8 of them, over **Sensor Wire**:

1. Install the module in a bay. Its cartridge has a small connector.
2. With Sensor Wire in hand, right-click the connector, then right-click block faces to route the wire (it follows surfaces on a 1/16 grid; hold **Left Alt** for straight L-shaped runs), then right-click a sensor's connector (the brass nub at its back). Sneak-right-click the air to cancel a run.
3. For more sensors, start a new run at a sensor and finish it by right-clicking the middle of an existing wire: that makes a junction, so all the sensors share one bus back to the module. Right-click the end of a wire to keep extending it.
4. Sneak-right-click a wire with shears to remove it. Right-click it twice with shears (not sneaking) to cut out the part between the two clicks. Right-click a wire with dye to recolour it; a dye in your off hand colours new wires.

Wires move with a Sable / Create Aeronautics structure when it assembles or disassembles. A wire stretched between the structure and the world is cut instead.

**Frames.** A lidar sweeps around the structure's up axis. Azimuth 0 is the sensor's forward, which is out of the wall for a wall mount, or the direction you looked when placing it on a floor or ceiling. Positive azimuth turns left (counter-clockwise from above) and positive elevation is up. Mounts and points use the **computer frame**: x is the way the computer's screen faces, y is to its left, z is up, the origin is the computer block's centre, in blocks. The computer and its sensors move together, so mounts never change while driving. Rays hit terrain, other structures, your own vehicle and entities, up to 64 blocks. There is no vehicle pose or odometry: that's for your program to work out.

Sensors are named `lidar_1`, `lidar_2`, ... in the order the module first sees them, or by an anvil name on the sensor item. Names, scan settings and "running" survive reloads, moves and taking the module out.

| Method | Description |
|--------|-------------|
| `names()` / `list()` | Sensor names / `[{name, type, mount, running}]` |
| `mount(name)` | `{x, y, z, yaw, mount, block, facing}` in the computer frame (`yaw` in degrees) |
| `configure(name, {az_min, az_max, az_steps, el_min, el_max, rows, range})` | Scan pattern in degrees and blocks; any subset of keys. Default: 360 columns, 1 row, 32 blocks |
| `get_config(name)` | The scan pattern |
| `scan(name)` | Take one scan; returns the `seq` it will have |
| `start(name)` / `stop(name)` | Scan continuously |
| `get_scan(name)` | Latest scan: `{seq, start_tick, end_tick, rows, columns, config, hits, ranges}`; `ranges` is little-endian float32 bytes, row-major, `inf` = no hit |
| `get_points(name)` | Latest scan's hits as float32 `x, y, z` triples in the computer frame |
| `get_seq(name)` | Latest finished scan's `seq` (0 = none) |
| `rename(name, new)` | Rename a sensor (sticks to its mounting spot) |
| `max_sensors()` / `max_rays()` | `8` / `8192` rays per scan (`az_steps * rows`) |

**Events:** `("sensor_attach", name, sensor, "lidar")`, `("sensor_detach", name, sensor)` and `("lidar_scan", name, sensor, seq)` when a scan finishes.

The server casts at most 4096 lidar rays per tick in total, so a big scan takes a few ticks. That's like a spinning lidar: a scan taken while moving is slightly skewed. `start_tick` and `end_tick` say when.

```python
import sensors

hub = sensors.find()
front = hub.lidar("lidar_1")
front.configure(az_min=-60, az_max=60, az_steps=121, el_min=-10, el_max=10, rows=3, range=24)
front.start()

scan = front.wait()
while True:
    near = scan.nearest()                      # (range, row, column) or None
    if near and near[0] < 3:
        print("obstacle at", scan.azimuth(near[2]), "degrees")
    for x, y, z in front.points():             # computer frame
        pass                                   # e.g. mark an occupancy grid
    scan = front.wait(after=scan.seq)
```

| Item | Recipe |
|------|--------|
| **Sensor Wire** (x8) | copper ingot + redstone + string |
| **Lidar Sensor** | `_, glass pane, _` / `redstone, comparator, redstone` / `_, iron ingot, _` |
| **Wired Sensor Module** | `sensor wire, comparator, sensor wire` / `iron ingot, redstone, iron ingot` |

**Try it:** `/ecm scenario spawn lidar_room` (op) builds a walled room 3 blocks south of you. The room has a computer with a Wired Sensor Module, a lidar wired to it, and `lidar_view.py` on the computer's disk. The scenario then runs the program, which draws a live top-down outline of what the lidar sees: `#` is a hit, `O` the lidar and `@` the computer. Walk into the room and you show up on the map. `/ecm scenario clear` removes it. The program is also in the mod jar at `evanscomputermod/scenarios/lidar_view.py`. It works on any computer with a lidar; `python lidar_view.py lidar_2` picks a sensor, and `FACING_VIEWER = False` puts the computer's forward at the top, for a car.

The wire system is ported from [PowerGrid](https://github.com/patryk3211/PowerGrid) (Apache-2.0); see `NOTICE`.

## Shell Commands

| Command | Usage | Description |
|---------|-------|-------------|
| `help` | `help` | List all commands |
| `clear` | `clear` | Clear the screen |
| `ls` | `ls` | List saved files |
| `cat` | `cat <file>` | Print file contents |
| `edit` | `edit <file>` | Open the text editor |
| `rm` | `rm <file>` | Delete a file |
| `echo` | `echo <text>` | Print text |
| `python` | `python` | Start the Python REPL |
| `python` | `python <file>` | Run a Python script |
| `ifconfig` | `ifconfig` | Show all network interfaces |
| `ifconfig` | `ifconfig eth0 10.0.0.1/24` | Configure interface with CIDR |
| `ifconfig` | `ifconfig eth0 up/down` | Bring interface up/down |
| `ip` | `ip addr` | Show/manage interface addresses |
| `ip` | `ip route` | Show/manage routing table |
| `ip` | `ip link` | Show/manage link-layer info |
| `ping` | `ping <ip> [-n <count>]` | Send ICMP echo requests |
| `nslookup` | `nslookup <hostname>` | DNS lookup |
| `resolvectl` | `resolvectl status` | Show DNS configuration |
| `resolvectl` | `resolvectl dns <iface> <server>` | Set DNS server |
| `resolvectl` | `resolvectl query <hostname>` | Resolve hostname |
| `httpd` | `httpd <port>` | Start HTTP file server |
| `curl` | `curl [options] <url>` | HTTP client |

**Ctrl+T** will terminate any running program and return to the shell.

---

### `ifconfig` — Interface Configuration

Show and configure network interfaces. Each computer has 6 built-in interfaces (eth0-eth5), one per face of the Terminal block.

**Show all interfaces:**
```
/ > ifconfig
eth0: flags=<UP>  mtu 1500
      ether 02:00:00:00:00:00
      inet 10.0.0.1/24

eth1: flags=<UP>  mtu 1500
      ether 02:00:00:00:00:01

eth2: flags=<DOWN>  mtu 1500
      ether 02:00:00:00:00:02
...
```

**Show a specific interface:**
```
/ > ifconfig eth0
eth0: flags=<UP>  mtu 1500
      ether 02:00:00:00:00:00
      inet 10.0.0.1/24
```

**Set IP address (CIDR notation):**
```
/ > ifconfig eth0 10.0.0.1/24
eth0: inet 10.0.0.1/24
```
This automatically adds a connected route (e.g. `10.0.0.0/24 dev eth0`) to the routing table.

**Bring interface up/down:**
```
/ > ifconfig eth0 down
Link down.
/ > ifconfig eth0 up
Link up.
```

**Host 802.1Q tagging (per interface):**
```
/ > ifconfig eth0 vlan 100
VLAN set to 100.
/ > ifconfig eth0 vlan off
eth0: VLAN tagging off.
```
A tagged interface sends every frame with the VLAN tag and only accepts
frames carrying it — use it to connect a computer to a switch trunk port.
Interface settings (address, VLAN, down) are saved to `/network.cfg` and
restored at boot. For switching between ports, see the `switch` command
and [`switch_instructions.md`](switch_instructions.md).

---

### `ip` — Network Configuration

Linux-like `ip` command with three subcommands: `addr`, `route`, and `link`.

#### `ip addr` — Address Management

**Show all addresses:**
```
/ > ip addr
eth0: flags=<UP>  mtu 1500
      ether 02:00:00:00:00:00
      inet 10.0.0.1/24

eth1: flags=<UP>  mtu 1500
      ether 02:00:00:00:00:01
      inet 192.168.1.1/24
...
```

**Add an address to an interface:**
```
/ > ip addr add 10.0.0.1/24 dev eth0
Added 10.0.0.1/24 to eth0
```

**Remove an address from an interface:**
```
/ > ip addr del 10.0.0.1/24 dev eth0
Removed address from eth0
```

#### `ip route` — Routing Table

Supports longest-prefix-match routing with up to 16 entries.

**Show routes:**
```
/ > ip route
10.0.0.0/24 dev eth0 scope link
192.168.1.0/24 dev eth1 scope link
default via 10.0.0.254 dev eth0
172.16.0.0/16 via 192.168.1.254 dev eth1
```

Routes with "scope link" are connected routes — auto-added when an interface is configured with `ifconfig` or `ip addr add`.

**Add a default route:**
```
/ > ip route add default via 10.0.0.254 dev eth0
Default route added.
```

**Add a specific route:**
```
/ > ip route add 172.16.0.0/16 via 192.168.1.254 dev eth1
Route 172.16.0.0/16 added.
```

**Delete a route:**
```
/ > ip route del default
Default route deleted.
/ > ip route del 172.16.0.0/16
Route deleted.
```

#### `ip link` — Link-Layer Management

**Show all links:**
```
/ > ip link
eth0: <UP> mtu 1500
    link/ether 02:00:00:00:00:00
eth1: <UP> mtu 1500
    link/ether 02:00:00:00:00:01
...
```

**Set link state:**
```
/ > ip link set eth0 down
Link down.
/ > ip link set eth0 up
Link up.
```

---

### `ping` — ICMP Echo

```
/ > ping 10.0.0.2
PING 10.0.0.2 - continuous
Reply from 10.0.0.2: time=110ms seq=0
Reply from 10.0.0.2: time=10ms seq=1
Reply from 10.0.0.2: time=10ms seq=2
Reply from 10.0.0.2: time=10ms seq=3
...
```

**With fixed packet count:**
```
/ > ping 10.0.0.2 -n 2
PING 10.0.0.2 - 2 packets
Reply from 10.0.0.2: time=85ms seq=0
Reply from 10.0.0.2: time=10ms seq=1
--- 10.0.0.2 ping statistics ---
2 packets sent, 2 received
```

The first ping to a new host is slower because it triggers ARP resolution. By default, ping runs until interrupted (Ctrl+T). Timeout is 2 seconds per packet.

---

### `nslookup` — DNS Lookup

```
/ > nslookup example.com
Server: 8.8.8.8
Name:    example.com
Address: 93.184.216.34
```

Requires a DNS server to be configured (see `resolvectl` below).

---

### `resolvectl` — DNS Configuration

Lightweight implementation of Linux's `resolvectl` for managing DNS servers.

**Show DNS status:**
```
/ > resolvectl status
Global DNS: 8.8.8.8

Link eth0 (UP):
    Address: 10.0.0.1/24
    DNS: 8.8.8.8
Link eth1 (UP):
    DNS: 8.8.8.8
...
```

**Set DNS server:**
```
/ > resolvectl dns eth0 8.8.8.8
DNS server set to 8.8.8.8
```

The interface name is accepted for Linux compatibility but the DNS server is set globally (all interfaces share the same DNS). Multiple servers can be specified; the first is used.

**Query a hostname:**
```
/ > resolvectl query example.com
Resolving example.com via 8.8.8.8...
example.com -> 93.184.216.34
```

**Show current DNS (no server arg):**
```
/ > resolvectl dns
Global DNS: 8.8.8.8
```

DNS configuration is automatically persisted to `network.cfg` and restored on reboot.

---

## Python

The terminal embeds a full Python interpreter. Start it with `python` for an interactive REPL, or `python script.py` to run a file.

```
> python
Python REPL (RustPython)
Type 'exit()' or 'quit()' to return to shell
>>> print("Hello, Minecraft!")
Hello, Minecraft!
>>> exit()
```

Two built-in modules are available: `terminal` and `net`.

---

### `terminal` Module

#### Text Output

```python
import terminal

terminal.write("no newline")
terminal.println("with newline")
terminal.clear()
```

#### Cursor and Screen

```python
terminal.set_cursor(0, 0)        # Move cursor (x, y), 0-based
width = terminal.get_width()     # 80
height = terminal.get_height()   # 24
```

#### Sleep

```python
terminal.sleep(1.0)    # seconds, supports fractional values
terminal.sleep(0.25)
```

#### File System

Each terminal has its own persistent storage. Files survive across sessions.

```python
# Write and read
terminal.write_file("greeting.txt", "Hello!")
content = terminal.read_file("greeting.txt")   # "Hello!" or None if missing

# Check and list
terminal.file_exists("greeting.txt")            # True
terminal.file_size("greeting.txt")              # 6 (bytes) or None
files = terminal.list_files()                   # newline-separated string

# Delete
terminal.delete_file("greeting.txt")            # True/False
```

#### Redstone

Terminals can read and write redstone signals (0-15) on all six sides. Sides are relative to the terminal's facing direction.

```python
import terminal

# Side constants
terminal.DOWN    # 0
terminal.UP      # 1
terminal.FRONT   # 2 (screen side)
terminal.BACK    # 3
terminal.LEFT    # 4
terminal.RIGHT   # 5

# Set output power level (0-15)
terminal.set_redstone(terminal.BACK, 15)   # Full power behind terminal
terminal.set_redstone(terminal.BACK, 0)    # Off

# Read input power level (0-15)
power = terminal.get_redstone(terminal.BACK)
levels = terminal.get_all_redstone()   # [DOWN, UP, FRONT, BACK, LEFT, RIGHT]
```

#### Events

Hardware events reach programs through the [`peripheral`](#peripheral-module) module's `pull_event()`. For the computer's own redstone sides, poll `get_redstone()` / `get_all_redstone()`.

---

### `peripheral` Module

Blocks next to the computer and modules in its bays (see [Modules and Peripherals](#modules-and-peripherals)).

```python
import peripheral

peripheral.names()                  # ['left', 'left_bay_1']
peripheral.attached()               # [('left', 'redstone_link'), ('left_bay_1', 'redstone_link')]
peripheral.get_type("left")         # 'redstone_link' (None if nothing is there)
peripheral.get_methods("left")      # ['get_channel_count', 'set_channel', ...]

link = peripheral.wrap("left_bay_1")        # None if nothing is attached there
link = peripheral.find("redstone_link")     # first of that type, or None
links = peripheral.find_all("redstone_link")
link.set_output(0, 15)                      # methods are attributes
peripheral.call("left_bay_1", "set_output", 0, 15)   # same thing

try:
    link.set_output(99, 1)
except peripheral.PeripheralError as e:
    print(e)                                # channel must be 0-63, got 99
```

**Events** are tuples `(event, attachment, *args)`:

```python
event = peripheral.pull_event()                         # wait for any event
event = peripheral.pull_event("redstone_link")          # skip other events
event = peripheral.pull_event("redstone_link", timeout=5)   # None after 5 s
```

| Event | Arguments |
|-------|-----------|
| `peripheral` | `name, type`: something was attached |
| `peripheral_detach` | `name`: it went away |
| `redstone_link` | `name, channel, power, old` |

Each program has its own event queue, created on its first `peripheral` call, so start listening (for example call `find()`) before you need the events. Queues hold 256 events; the oldest are dropped if a program stops reading. Values passed to and from peripherals can be `None`, `bool`, `int`, `float`, `str`, `bytes`, `list`, `tuple` and `dict`.

---

### `net` Module

The `net` module provides a full TCP/IP networking stack built entirely inside the WASM OS. Computers can communicate with each other over a virtual Ethernet LAN, and optionally reach the real internet through a TAP bridge.

#### Network Architecture

```
Computer A ──┐
Computer B ──┼── Ethernet Hub (in-memory) ──┬── frames routed by MAC
Computer C ──┘                              └── TAP Bridge ── Linux kernel ── Internet
```

The host exposes 6 raw ethernet frame primitives (one per interface). Everything above Layer 2 — ARP, IPv4, ICMP, UDP, TCP, DNS, HTTP — is implemented from scratch in the Rust OS.

| Layer | Protocol | Implementation |
|-------|----------|----------------|
| 2 | Ethernet | Frame parse/serialize, 6-byte MAC addressing, 802.1Q VLAN tagging |
| 2.5 | ARP | 32-entry cache (5-min TTL), request/reply, gratuitous ARP |
| 3 | IPv4 | Header with checksum, routing table (local subnet + default gateway) |
| 3 | ICMP | Echo request/reply (ping) |
| 4 | UDP | 8 sockets, ring buffers, ephemeral port allocation |
| 4 | TCP | Full RFC 793 state machine, 8 connections, 4KB buffers, retransmission |
| 7 | DNS | A record query/response, compression pointer support |
| 7 | HTTP/1.0 | Request parser, response builder, client and server |

#### Multi-Interface Architecture

Each computer has **5 built-in network interfaces** (one per non-screen face of the Terminal block). The screen face has no interface and does not connect to cables. Attaching **Network Interface** blocks replaces the occupied face's interface with 5 new ones from the Interface block's free faces.

Interface discovery happens at boot: the terminal scans adjacent blocks for Interface blocks, walks chains, and counts free faces. Each face becomes an ethN interface with a unique MAC.

**Interface math:**
- Bare terminal: **5 interfaces**
- Terminal + 1 Interface block: **4 + 5 = 9 interfaces**
- Terminal + 2 chained Interface blocks: **4 + 5 + 5 = 14 interfaces**

Each computer gets unique MAC addresses derived from its UUID + interface index. In the simulator, MACs are `02:II:5e:00:NN:NN` where II is the interface index and NNNN the node number (from 1).

#### Interface Management (Python)

```python
import net

# List all interfaces — returns list of dicts
ifaces = net.interfaces()
for i in ifaces:
    print(f"{i['name']}: mac={i['mac']} ip={i['ip']}/{i['prefix_len']} "
          f"up={i['link_up']}")

# Configure interface IP (CIDR notation)
net.iface_set("eth0", "10.0.0.1/24")      # Automatically adds connected route

# Bring interface up/down
net.iface_up("eth0")
net.iface_down("eth1")
```

#### Routing (Python)

```python
import net

# Add routes
net.route_add("default", "10.0.0.254", "eth0")           # Default gateway
net.route_add("192.168.1.0/24", "10.0.0.1", "eth1")      # Specific subnet

# List all routes — returns list of dicts
for r in net.routes():
    print(f"{r['destination']}/{r['prefix_len']} via {r['gateway']} dev {r['dev']}")

# Delete routes
net.route_del("default")
net.route_del("192.168.1.0/24")
```

#### DNS (Python)

```python
import net

net.dns_set("8.8.8.8")        # Set DNS server
dns = net.dns_get()            # Get current DNS server → "8.8.8.8"
```

#### Configuration Persistence

Network configuration is automatically saved to `network.cfg` when changes are made. The file format:

```
iface eth0 10.0.0.1/24
iface eth1 192.168.1.1/24
dns 8.8.8.8
route default via 10.0.0.254 dev eth0
route 172.16.0.0/16 via 192.168.1.254 dev eth1
```

Configuration is restored automatically on reboot.

#### 802.1Q VLANs

VLAN tagging is handled by the in-kernel L2 switch built-in (`switch` command),
not by individual NIC configuration. Hosts connect to access ports on the
switch, the switch tags/untags frames at the port boundary, and broadcast
domains are isolated per VLAN. See the `switch` documentation for the full
VLAN configuration sub-shell (`vlan <id>`, `interface <port>`,
`vlan access`, `vlan trunk native`, `vlan trunk allowed`, `show vlan`).

#### ICMP (Ping)

```python
import net

rtt = net.ping("10.0.0.2")     # Returns RTT in milliseconds
print(f"Round trip: {rtt}ms")   # Round trip: 12ms
```

Shell:
```
/ > ping 10.0.0.2 -n 4
PING 10.0.0.2 - 4 packets
Reply from 10.0.0.2: time=110ms seq=0
Reply from 10.0.0.2: time=10ms seq=1
Reply from 10.0.0.2: time=10ms seq=2
Reply from 10.0.0.2: time=10ms seq=3
--- 10.0.0.2 ping statistics ---
4 packets sent, 4 received
```

The first ping is slower because it triggers ARP resolution.

#### DNS

```python
import net

ip = net.resolve("example.com")
print(ip)  # "93.184.216.34"
```

Shell:
```
/ > nslookup example.com
Server: 8.8.8.8
Name:    example.com
Address: 93.184.216.34
```

DNS queries are sent via UDP to the configured DNS server (port 53).

#### UDP Sockets

```python
import net

# Open a socket bound to port 9000
sock = net.udp_open(9000)

# Send a datagram
net.udp_send(sock, "10.0.0.2", 8080, b"Hello!")

# Receive (blocks up to 5 seconds, returns tuple or None)
result = net.udp_recv(sock)
if result:
    data, src_ip, src_port = result
    print(f"Received {data} from {src_ip}:{src_port}")

# Close
net.udp_close(sock)
```

Up to 8 UDP sockets can be open simultaneously. Each has a 2KB receive ring buffer.

#### TCP Connections

**Client:**
```python
import net

# Connect to a server (blocks until established or timeout)
conn = net.tcp_connect("10.0.0.2", 8080)

# Send data
net.tcp_send(conn, b"GET / HTTP/1.0\r\nHost: 10.0.0.2\r\n\r\n")

# Receive data (blocks until data available or timeout)
data = net.tcp_recv(conn, 4096)
print(data)

# Close connection (sends FIN)
net.tcp_close(conn)
```

**Server:**
```python
import net

# Listen on a port
listener = net.tcp_listen(8080)

# Accept a connection (blocks until a client connects)
conn = net.tcp_accept(listener)

# Read request
data = net.tcp_recv(conn, 4096)
print(f"Received: {data}")

# Send response
net.tcp_send(conn, b"Hello from server!")

# Close
net.tcp_close(conn)
net.tcp_close(listener)
```

TCP features:
- Full 3-way handshake (SYN, SYN-ACK, ACK)
- Reliable delivery with retransmission (exponential backoff: 1s, 2s, 4s, 8s, 16s)
- Sliding window flow control (4KB send/receive buffers)
- Graceful close (FIN/FIN-ACK) and TIME_WAIT
- Up to 8 simultaneous connections
- MSS: 1460 bytes (MTU 1500 - IP 20 - TCP 20)

#### HTTP Client (`curl`)

**Python:**
```python
import net

# GET request — returns dict with status, body, headers
resp = net.http_get("http://10.0.0.2:8080/index.txt")
print(resp['status'])   # 200
print(resp['body'])     # "Hello World!"
print(resp['headers'])  # {'Content-Type': 'text/plain', ...}

# POST request
resp = net.http_post("http://10.0.0.2:8080/data.txt", b"file contents here")
print(resp['status'])   # 200
```

**Shell:**
```
/ > curl http://10.0.0.2:8080/
<html>...directory listing...</html>

/ > curl -v http://10.0.0.2:8080/hello.txt
> GET /hello.txt HTTP/1.0
> Host: 10.0.0.2:8080
>
< HTTP/1.0 200 OK
< Server: TerminalOS/1.0
< Content-Type: text/plain
< Content-Length: 13
<
Hello World!

/ > curl -X POST -d "new content" http://10.0.0.2:8080/uploaded.txt
OK
```

`curl` flags:
| Flag | Description |
|------|-------------|
| `-v` | Verbose — show request and response headers |
| `-X METHOD` | Set HTTP method (GET, POST, etc.) |
| `-d DATA` | Set request body (implies POST). Supports quoted strings. |
| `-H "Key: Value"` | Add a custom header |

URL format: `http://host:port/path` (port defaults to 80).

#### HTTP Server (`httpd`)

```
/ > httpd 8080
HTTP server listening on port 8080. Ctrl+T to stop.
GET /
GET /hello.txt
POST /upload.txt
```

The `httpd` command starts an HTTP/1.0 file server that serves the computer's virtual filesystem:

| Route | Method | Behavior |
|-------|--------|----------|
| `GET /` | GET | HTML directory listing with links |
| `GET /<file>` | GET | Serve file contents with appropriate Content-Type |
| `POST /<file>` | POST | Write request body to file, returns "OK" |

Content-Type detection:
| Extension | Content-Type |
|-----------|-------------|
| `.html`, `.htm` | `text/html` |
| `.json` | `application/json` |
| `.py` | `text/x-python` |
| (other) | `text/plain` |

The server handles one request at a time (single-threaded). Press **Ctrl+T** to stop.

#### Addressing and Routing

Each computer has a MAC address (6 bytes, derived from its UUID) and an IPv4 address (configured manually via `ifconfig set` or `net.configure()`).

**MAC addressing:** Frames are delivered by the Ethernet hub based on destination MAC. Broadcast frames (`ff:ff:ff:ff:ff:ff`) go to all computers. ARP resolves IP → MAC automatically.

**IP routing:** The OS uses a simple routing table:
- If the destination IP is on the same subnet (matching under the subnet mask), send directly via ARP
- Otherwise, send to the default gateway

**Inter-computer communication:** All computers on the same server share an Ethernet hub. They can ping, send UDP, establish TCP connections, and run HTTP servers/clients to each other — all using real protocol implementations.

---

## Networking — Technical Details

### Cables, faces and segments

Every face of a computer (plus each attached Network Interface block) is its own NIC with its own MAC. A connected mesh of cable blocks is one Ethernet **segment**: a frame sent by a NIC is offered to every other NIC on that segment, and each NIC's filter accepts its own MAC, broadcast, multicast, or everything when promiscuous (`NetworkHub.java`, segments computed by `CableNetworkManager.java`). Computers are not part of the cable mesh, so two faces of one computer are different segments — which is what lets a computer act as a switch or router. `ifconfig <iface> down` disables the NIC (no TX/RX); the kernel sees carrier through `net_get_link_state`. Each NIC queues up to 256 frames (oldest dropped) and raises one coalesced network interrupt at a time.

### Kernel networking

`Net` (`rust/operating-system/rust/src/net/`) is the single owner of the NICs; nothing else receives frames. Each received frame goes to exactly one consumer: the L2 bridge if the switch is running and the port is an access/trunk port, otherwise the host stack.

- **`ecm-net`** (`rust/crates/ecm-net`) — a sans-IO IPv4 stack: no globals, no host calls, time passed in. Multiple interfaces with per-interface 802.1Q tagging, longest-prefix routing, ARP with pending queues, ICMP, UDP, TCP (retransmission, windows, out-of-order reassembly, fast retransmit, RFC 5961 checks), DNS client, loopback. Validated against hostile input (fuzz tests) and lossy links (byte-exact 1 MiB transfers under loss and reordering).
- **Socket syscalls** (`net/ipc.rs`) — child programs use BSD-style sockets (`ecm-host-abi/src/socket.rs`). Each syscall is answered immediately or with `IPC_PENDING`; per-socket timeouts (`SO_RCVTIMEO`, connect 10 s, DNS 6 s) live in the kernel. Results are `[status][payload]`.
- **Netlink** (`net/netlink.rs`) — `ifconfig` and `ip` talk rtnetlink to the kernel. Every change is persisted to `/network.cfg` and restored at boot.
- **Switch** — see [`switch_instructions.md`](switch_instructions.md). The switch is a kernel service: `switch on` runs it in the background, `switch` opens its AOS-CX style CLI, and `interface vlan N` + `ip address` gives it a management address (SVI) reachable through the bridge.

### TAP Bridge — Real Internet Access

The TAP bridge connects segments that reach an **Internet Gateway** block to a real Linux TAP device. Unicast frames for MACs not on the segment, and all broadcast/multicast, are copied to the TAP; frames from the TAP appear on every gateway-connected segment.

```bash
sudo scripts/setup-tap.sh tap0      # create TAP + NAT rules (Linux, as root)
sudo scripts/teardown-tap.sh tap0   # remove them
```

On a Minecraft server the TAP is configured in the mod's config file (the Java mod uses a small helper process to open the device).

### Testing

| Layer | What | Command (from `rust/` unless noted) |
|---|---|---|
| Stack | `ecm-net` unit, stack-pair, TCP-under-loss and fuzz tests | `cargo test -p ecm-net` |
| Switch | `ecm-bridge` codecs, CLI, multi-bridge STP/LACP/LLDP/VLAN netsim, fuzz | `cargo test -p ecm-bridge` |
| Kernel | shell, jobs, IPC over real stacks, dispatcher (TCP through a switch, VLANs, trunks, STP loop) | `cargo test -p terminal-os` |
| ABI | built kernel vs. `abi/host-abi.toml` | `python3 scripts/check-abi.py` (repo root) |
| Java host | real kernel inside `ComputerInstance` on Chicory (boot, child programs, Ctrl+T, switch, SSH over loopback) | `./gradlew :26.1:test` (repo root; needs the kernel and programs built) |
| In-world | GameTests: real terminals + cables — ping and SSH between two computers | `./gradlew :26.1:runGameTestServer -PgameTestNamespaces=ecm_network` |
| System | real kernel + programs in multi-node topologies, virtual clock, scenarios | see [`rust/simulator/README.md`](rust/simulator/README.md) |

How to choose and run tests (and the receipts they produce): [`TESTING.md`](TESTING.md), via `scripts\Test.ps1`.

---

## Examples

### Ping Another Computer

```python
import net

net.configure("10.0.0.1", "255.255.255.0", "10.0.0.254", "8.8.8.8")
rtt = net.ping("10.0.0.2")
print(f"Ping: {rtt}ms")
```

### HTTP File Server

Start a web server that serves files from the computer's filesystem:

```
/ > echo "Hello from Minecraft!" > index.txt
/ > httpd 8080
HTTP server listening on port 8080. Ctrl+T to stop.
```

From another computer:
```
/ > curl http://10.0.0.1:8080/index.txt
Hello from Minecraft!
```

### Python Web Server with Custom Routes

```python
import net
import terminal

net.configure("10.0.0.1", "255.255.255.0", "10.0.0.254", "8.8.8.8")

listener = net.tcp_listen(8080)
terminal.println("Server ready on :8080")

while True:
    conn = net.tcp_accept(listener)
    data = net.tcp_recv(conn, 4096)

    # Parse the request line
    request = data.decode()
    path = request.split(" ")[1] if " " in request else "/"

    if path == "/status":
        body = '{"status": "online", "redstone": ' + str(terminal.get_all_redstone()) + '}'
        response = f"HTTP/1.0 200 OK\r\nContent-Type: application/json\r\nContent-Length: {len(body)}\r\n\r\n{body}"
    else:
        body = "<h1>Minecraft Computer</h1><p>Try /status</p>"
        response = f"HTTP/1.0 200 OK\r\nContent-Type: text/html\r\nContent-Length: {len(body)}\r\n\r\n{body}"

    net.tcp_send(conn, response.encode())
    terminal.sleep(0.1)
    net.tcp_close(conn)
```

### UDP Chat Between Computers

**Computer 1 (sender):**
```python
import net
net.configure("10.0.0.1", "255.255.255.0", "10.0.0.254", "8.8.8.8")
sock = net.udp_open(9000)
net.udp_send(sock, "10.0.0.2", 9000, b"Hello from Computer 1!")
net.udp_close(sock)
```

**Computer 2 (receiver):**
```python
import net
import terminal
net.configure("10.0.0.2", "255.255.255.0", "10.0.0.254", "8.8.8.8")
sock = net.udp_open(9000)
terminal.println("Waiting for message...")
result = net.udp_recv(sock)
if result:
    data, ip, port = result
    terminal.println(f"From {ip}:{port}: {data.decode()}")
net.udp_close(sock)
```

### Fetch a Web Page from the Real Internet

Requires TAP bridge setup (see Networking — Technical Details):

```python
import net

net.configure("10.0.0.1", "255.255.255.0", "10.0.0.254", "8.8.8.8")

# Resolve domain name
ip = net.resolve("example.com")
print(f"Resolved: {ip}")

# Fetch the page
resp = net.http_get(f"http://{ip}/")
print(f"Status: {resp['status']}")
print(resp['body'][:200])
```

### File Transfer Between Computers

**Upload a file to another computer's HTTP server:**
```python
import net

net.configure("10.0.0.1", "255.255.255.0", "10.0.0.254", "8.8.8.8")

# Read local file and upload to Computer 2
import terminal
content = terminal.read_file("important.txt")
if content:
    resp = net.http_post("http://10.0.0.2:8080/backup.txt", content.encode())
    print(f"Upload: {resp['status']}")
```

---

### Redstone Pulse

```python
import terminal

def pulse(side, duration=0.5):
    terminal.set_redstone(side, 15)
    terminal.sleep(duration)
    terminal.set_redstone(side, 0)

pulse(terminal.BACK)
terminal.println("Pulse sent!")
```

### Redstone Clock

```python
import terminal

terminal.println("Redstone clock running. Ctrl+T to stop.")
while True:
    terminal.set_redstone(terminal.BACK, 15)
    terminal.sleep(0.5)
    terminal.set_redstone(terminal.BACK, 0)
    terminal.sleep(0.5)
```

### Saving and Running Scripts

From the shell:
```
> edit hello.py
```
Write your script in the editor, save, then:
```
> python hello.py
```

Or create files from Python directly:
```python
import terminal

terminal.write_file("startup.py", """
import terminal
terminal.println("Computer booted!")
terminal.set_redstone(terminal.UP, 15)
""")
```

### Custom Importable Modules

Python files saved to the terminal's filesystem can be imported normally:

```python
import terminal

# Create a module
terminal.write_file("utils.py", """
def greet(name):
    return f"Hello, {name}!"

PI = 3.14159
""")
```

Then in the REPL or another script:
```python
import utils

print(utils.greet("Steve"))   # Hello, Steve!
print(utils.PI)               # 3.14159
```

This works because the OS installs a custom import hook that checks the virtual filesystem before falling back to standard imports.

### Redstone Input Monitor

Poll the computer's redstone inputs and mirror the back input to the front:

```python
import terminal

terminal.println("Redstone monitor running. Ctrl+T to stop.")
last = None
while True:
    sides = terminal.get_all_redstone()     # [DOWN, UP, FRONT, BACK, LEFT, RIGHT]
    if sides != last:
        terminal.set_cursor(0, 2)
        terminal.write(f"Input levels: {sides}   ")
        terminal.set_redstone(terminal.FRONT, sides[terminal.BACK])
        last = sides
    terminal.sleep(0.1)
```

### Redstone Link Remote

Watch 16 Create link frequencies (one per wool colour) and log every change:

```python
import peripheral

COLORS = ["white", "orange", "magenta", "light_blue", "yellow", "lime", "pink", "gray",
          "light_gray", "cyan", "purple", "blue", "brown", "green", "red", "black"]
link = peripheral.find("redstone_link")
for ch, color in enumerate(COLORS):
    link.set_channel(ch, "minecraft:%s_wool" % color, "minecraft:%s_wool" % color, "rx")

while True:
    _, _, ch, power, old = peripheral.pull_event("redstone_link")
    print("%-10s %2d -> %2d" % (COLORS[ch], old, power))
```

---

## Mod Integration API

EvansComputerMod is designed for other mods to extend. Blocks can offer computers a **peripheral**, and items can be **modules** that install inside a computer, with a few annotations and no WASM knowledge. You can also embed a full computer into your own blocks, entities, or items.

### Peripherals: exposing a block to computers

A peripheral is an `IPeripheral`. The easy way to write one is to extend `AnnotatedPeripheral` and mark the callable methods:

```java
import com.example.evanscomputermod.api.peripheral.*;

public class LampPeripheral extends AnnotatedPeripheral {
    private final LampBlockEntity lamp;

    public LampPeripheral(LampBlockEntity lamp) { this.lamp = lamp; }

    @Override public String getType() { return "lamp"; }

    @PeripheralMethod(description = "Turn the lamp on or off")
    public void setLit(boolean lit) { lamp.setLit(lit); }            // lamp.set_lit(True)

    @PeripheralMethod(description = "Set the colour")
    public void setColor(int rgb) throws PeripheralException {
        if (rgb < 0 || rgb > 0xFFFFFF) throw new PeripheralException("colour must be 0x000000-0xFFFFFF");
        lamp.setColor(rgb);
    }

    @PeripheralMethod(description = "Whether the lamp is on")
    public boolean isLit() { return lamp.isLit(); }
}
```

Expose it on your block entity through the peripheral capability:

```java
modBus.addListener((RegisterCapabilitiesEvent event) ->
        event.registerBlockEntity(PeripheralCapability.PERIPHERAL, MY_LAMP_BE.get(),
                (be, side) -> be.getPeripheral()));   // return the same object each time
```

A computer finds it on its next neighbour update, under the name of the side it's on. Optionally register the type so the visual editor gets blocks for it: `PeripheralTypes.register("lamp", "Colour lamp", LampPeripheral.class)` in common setup.

- **Threading.** Methods run on the **server thread** by default, so they can touch the world. The calling program waits for the next server tick. `@PeripheralMethod(mainThread = false)` runs a thread-safe method directly on the program's thread.
- **Arguments.** Parameters are converted from the program's values: `int`, `long`, `double`, `float`, `boolean` (and boxes), `String`, `byte[]`, `List`, `Map`, `Object`. A leading `IComputerAccess` parameter is injected. Boxed and reference parameters are optional (a missing argument is `null`). Bad arguments get a clear error without calling the method.
- **Results.** Return `null`, numbers, booleans, strings, `byte[]`, enums (sent as lower-case names), `List`/arrays/collections and `Map`s.
- **Errors.** Throw `PeripheralException` to raise `peripheral.PeripheralError` in the program with your message.
- **Events.** `attach(IComputerAccess)` / `detach(...)` tell you which computers can see the peripheral; `computer.queueEvent("lamp_toggled", true)` delivers `("lamp_toggled", name, True)` to that computer's programs.
- **`isSame`.** When a computer rescans its neighbours, a peripheral that `isSame()` as the attached one stays attached. The default is identity, so return a stable object from the capability.

### Modules: peripherals inside the computer

A module item implements `IComputerModuleItem` and creates an `IComputerModule`, a peripheral with a lifecycle:

```java
public class LampModuleItem extends Item implements IComputerModuleItem {
    @Override
    public IComputerModule createModule(IModuleHost host, ItemStack stack, CompoundTag saved) {
        return new LampModule(host, saved.getInt("color"));
    }
    // getSlotVisual(stack): how it looks in the bay (GENERIC cartridge by default)
}

public class LampModule extends AnnotatedPeripheral implements IComputerModule {
    ...
    @Override public void onLoad() { /* live at host.getPos(): register with world systems */ }
    @Override public void onUnload() { /* leaving: removed, chunk unloaded, or about to move */ }
    @Override public void tick() { }
    @Override public void saveState(CompoundTag tag) { tag.putInt("color", color); }
}
```

- **State.** `saveState` is stored on the module's item stack, so it survives ejecting the module, breaking the computer and Sable moves. Call `host.markDirty()` when it changes.
- **Lifecycle.** A module can be loaded and unloaded several times, for example around a Sable / Create Aeronautics assembly. On a sub-level, `host.getPos()` is the position inside the sub-level's plot.

### Wire format

Programs reach peripherals through five host imports that WASI programs link from `env` (see `rust/crates/ecm-host-abi/src/peripheral.rs`):

| Import | Signature | Purpose |
|--------|-----------|---------|
| `periph_list` | `(buf, cap) -> len` | `[[name, type], ...]` |
| `periph_methods` | `(name, len, buf, cap) -> len` | `[type, [method, ...]]` |
| `periph_call` | `(name, len, method, len, args, len, buf, cap) -> len` | call a method with a LIST of arguments |
| `periph_wait_event` | `(filter, len, timeout_ms, buf, cap) -> len` | next event, 0 on timeout |
| `periph_take_pending` | `(buf, cap) -> len` | fetch a result that was bigger than `cap` |

Each call returns a frame `[status u8][value]` (0 ok, 1 error with a string message). Values use a tagged little-endian encoding: `0` nil, `1` str (u32 length + UTF-8), `2` i32, `3` i64, `4` f64, `5` bool, `6` list (u32 count + values), `7` map (u32 count + key/value pairs), `8` bytes. `PeripheralValues.java` and `peripheral.rs` implement both sides. The Python `peripheral` module is `peripheral.py` over the native `_peripheral` module.

---

### Embedding a Computer in Your Own Block/Entity/Item

The computer runtime is fully decoupled from the terminal block. You can embed a computer into any context by implementing `IComputerHost`.

```java
import com.example.evanscomputermod.api.*;
import com.example.evanscomputermod.computer.ComputerInstance;
import com.example.evanscomputermod.computer.ComputerRegistry;

public class DroneEntity extends Entity implements IComputerHost {
    private final UUID computerId = UUID.randomUUID();
    private ComputerInstance computer;

    // Required: identity and server access
    @Override public UUID getComputerId() { return computerId; }
    @Override public MinecraftServer getServer() { return level().getServer(); }
    @Override public void markDirty() { /* entity state changed */ }
    @Override public void syncToClients() { /* send updates to tracking players */ }

    // Optional capabilities — return null to disable
    @Override public ITerminalOutput getTerminalOutput() { return null; }  // headless
    @Override public IRedstoneProvider getRedstoneProvider() { return null; }
    @Override public IWorldAccess getWorldAccess() {
        return new IWorldAccess() {
            public Level getLevel() { return DroneEntity.this.level(); }
            public BlockPos getBlockPos() { return DroneEntity.this.blockPosition(); }
        };
    }
    @Override public IVisualProgramming getVisualProgramming() { return null; }

    public void startComputer() {
        computer = new ComputerInstance(this);
        computer.loadModule("terminal_os");
        computer.executeMain();
        computer.startWorkerThread();
        ComputerRegistry.register(this);
    }

    @Override public void remove(RemovalReason reason) {
        if (computer != null) { computer.close(); }
        ComputerRegistry.unregister(computerId);
        super.remove(reason);
    }
}
```

#### `IComputerHost` Capabilities

| Method | Returns | Required | Purpose |
|--------|---------|----------|---------|
| `getComputerId()` | `UUID` | Yes | Persistent identity for file storage |
| `getServer()` | `MinecraftServer` | Yes | Main thread scheduling |
| `markDirty()` | `void` | Yes | Signal state needs persistence |
| `syncToClients()` | `void` | Yes | Push display updates to clients |
| `getTerminalOutput()` | `ITerminalOutput` | No (null = headless) | 80x24 character display |
| `getRedstoneProvider()` | `IRedstoneProvider` | No (null = no redstone) | Redstone I/O |
| `getWorldAccess()` | `IWorldAccess` | No | World position access |
| `getVisualProgramming()` | `IVisualProgramming` | No (null = disabled) | Visual editor support |
| `getPeripheralHub()` | `PeripheralHub` | No (null = no peripherals) | Attached peripherals and modules |

---

## Building from Source

### Prerequisites

- Java 21
- Rust toolchain with `wasm32-unknown-unknown` target (`rustup target add wasm32-unknown-unknown`)
- Gradle (wrapper included)

### Build

```bash
./copy-jar.sh
```

This will:
1. Compile the Rust operating systems to WASM
2. Build the mod JAR (with WASM binaries bundled inside)
3. Copy the final JAR to the project root

The WASM files are extracted from the JAR automatically when the mod loads.

### Manual Build

```bash
# Build Rust WASM
cd operating-system/rust && cargo build --release
cd operating-system/simple && cargo build --release --target wasm32-unknown-unknown

# Copy WASM to wasm-bin/
mkdir -p wasm-bin
cp operating-system/rust/target/wasm32-unknown-unknown/release/terminal_os.wasm wasm-bin/
cp operating-system/simple/target/wasm32-unknown-unknown/release/simple.wasm wasm-bin/terminal.wasm

# Build mod
./gradlew build
```

## Shell-First Architecture

All user-facing I/O goes through `ShellInstance`. The shell abstracts whether output goes to the physical Minecraft terminal or an SSH channel. **This means every command and every Python script works identically over SSH.**

```
                    ┌─────────────────────┐
                    │   ShellInstance      │
                    │  ┌───────────────┐  │
                    │  │ OutputSink    │  │
  Local Terminal ◄──┤  │  ::Terminal   │  │
                    │  │  ::Buffer ────┼──┼──► SSH Channel
                    │  └───────────────┘  │
                    │  input_buf, cwd,    │
                    │  job_table, state   │
                    └─────────────────────┘
                              ▲
                    cmd_ls(), cmd_cat(), Python REPL, etc.
                    all call shell.print() / shell.read_line()
```

### Writing Shell Commands (Rust)

Every command function takes `shell: &mut ShellInstance`:

```rust
fn cmd_example(shell: &mut ShellInstance, args: &str) {
    shell.println("Hello from my command!");
    let input = shell.read_line("Enter something: ");
    shell.print(&format!("You said: {}\n", input));
}
```

**Do NOT use `terminal::print()` directly** — it bypasses the shell and won't work over SSH. The `terminal` module is `pub(crate)` (kernel-internal only).

### Writing Python Scripts

Python scripts use the `shell` module:

```python
import shell

# Output
shell.write("Hello ")
shell.println("World!")

# Input
name = shell.input("What is your name? ")
shell.println("Hello, " + name)

# Screen
shell.clear()
shell.set_cursor(0, 0)
w = shell.get_width()
h = shell.get_height()

# Files
shell.write_file("data.txt", "hello")
content = shell.read_file("data.txt")
if shell.file_exists("data.txt"):
    shell.delete_file("data.txt")
files = shell.list_files()
size = shell.file_size("data.txt")

# Sleep
shell.sleep(1.5)  # seconds

# Redstone
shell.set_redstone(shell.FRONT, 15)
power = shell.get_redstone(shell.BACK)
all_sides = shell.get_all_redstone()  # [down, up, front, back, left, right]

# Peripherals and their events: see the `peripheral` module
import peripheral
event = peripheral.pull_event(timeout=1)
```

All of the above works identically whether running locally or over SSH.

### SSH

- **`sshd [port]`** — Start SSH server (default port 22)
- **`ssh [user[:password]@]host[:port]`** — Connect to remote computer
- **`passwd`** — Set password for SSH authentication
- **`ssh-keygen`** — Generate/regenerate host keys

`ssh` and `sshd` use the computer's own network configuration through the kernel's socket API (set it with `ifconfig`/`ip`/`route`); they no longer read `NET_IP`/`NET_GATEWAY`/`NET_DNS`. `sshd` serves one connection at a time.

SSH sessions get their own `ShellInstance` with `OutputSink::Buffer`. All command output is captured and sent as SSH CHANNEL_DATA. Interactive commands (`passwd`, `python`) work over SSH via TCP-polling `read_line`.

#### SSH Virtual Terminal Protocol

The Minecraft terminal is a 2D character buffer, not a VT100 terminal -- it doesn't
interpret ANSI escape sequences. To support cursor-addressed programs (like the
editor) over SSH, we use a simple binary protocol embedded in SSH CHANNEL_DATA:

| Bytes | Host Function Called | Description |
|-------|---------------------|-------------|
| `0xFF 0x01 x y` | `terminal_set_cursor(x, y)` | Move cursor to position |
| `0xFF 0x02` | `terminal_clear()` | Clear screen and reset cursor |
| Any other bytes | `terminal_write(text)` | Write text at current cursor |

`0xFF` never appears in valid UTF-8, so there's no ambiguity with regular text.

The SSH server's ShellInstance emits these protocol bytes when commands call
`shell.set_cursor()` or `shell.clear()`. The SSH client parses the data stream,
extracting protocol commands and executing the corresponding host functions on
the client's terminal. This means the editor works identically over SSH -- the
remote terminal buffer is manipulated through the same host functions as local.

**Commands available over SSH**: All commands except `visual` (requires client GUI),
`sshd` (would nest server loops), and `httpd` (would nest blocking loops).

### Kernel Architecture

The Rust OS is a proper kernel supporting:

- **Multiprocessing**: WASI child processes, `ps`, `kill`, job control (`&`, `jobs`, `fg`, `bg`)
- **Pipes & Redirection**: `cmd1 | cmd2`, `>`, `>>`, `<`, `2>&1`
- **Virtual TTYs**: Per-TTY I/O buffers, foreground switching
- **File Descriptors**: FdTable with PipeFd, VfsFileFd, TerminalFd, SocketFd
- **Full TCP/IP Stack**: Ethernet, ARP, IPv4, ICMP, UDP, TCP, DNS, HTTP
- **SSH**: curve25519-sha256 KEX, Ed25519 host keys, ChaCha20-Poly1305 (all pure-Rust, compiled to WASM)
- **WASI Programs**: Drop `.wasm` files into `/bin/` and run them

### Host Functions (94 total)

| Category | Count | Namespace |
|----------|-------|-----------|
| Terminal, Filesystem, Redstone, Interrupts, Network, Modules | 32 | env |
| File Descriptors, Process, TTY, Sockets | 25 | env |
| WASI I/O + Stubs | ~34 | wasi_snapshot_preview1 |

### Testing

See [`TESTING.md`](TESTING.md): which tests cover which code, and the runner
(`scripts/Test.ps1`) that runs them and writes a receipt. To build and stage
the kernel and programs by hand: `./scripts/stage-wasm.sh`.

Simulator scenarios directly:

```bash
./scripts/run-scenarios.sh         # build kernel + programs, run every simulator scenario
./scripts/run-scenarios.sh stp     # only scenarios whose file name contains "stp"
./scripts/test-all.sh              # scenarios + simulator unit tests
./scripts/test-networking.sh       # cable/switch/VLAN/STP/LACP/TCP/fault scenarios
./scripts/test-switch.sh           # switch scenarios
./scripts/test-processes.sh        # shell, pipes, redirects, jobs, Ctrl+T
./scripts/test-ssh.sh              # sshd + ssh between two computers
```

### Simulator

```bash
cd rust
cargo run --release -p terminal-simulator -- --scenario simulator/scenarios/03_switch_three_hosts.toml
cargo run --release -p terminal-simulator -- --topology my-lab.toml    # interactive, one view per node
cargo run --release -p terminal-simulator -- --nodes 2                 # two unconnected computers
```

Topology files, the scenario step language, the virtual clock and fault
injection are described in [`rust/simulator/README.md`](rust/simulator/README.md).

## Installation

1. Install [NeoForge](https://neoforged.net/) for Minecraft 1.21.1
2. Download the mod JAR from [Releases](https://github.com/ewitulsk/EvanModCursor/releases)
3. Place it in your `mods/` folder

## License

MIT
