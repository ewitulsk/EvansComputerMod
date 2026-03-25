# EvansComputerMod

A Minecraft mod that adds in-world computer terminals powered by WebAssembly. The terminals run a Rust-based operating system with an embedded Python interpreter (RustPython), giving you a fully programmable computer inside Minecraft.

**Minecraft:** 1.21.1 | **Mod Loader:** NeoForge 21.1.77+

## Architecture

```
Python Script
    |
RustPython Interpreter (embedded in Rust OS)
    |
Rust Operating System (compiled to WASM)
    |  \
    |   Full TCP/IP Stack (Ethernet/ARP/IP/ICMP/UDP/TCP/DNS/HTTP)
    |                |
Wasmtime-Java    Ethernet Hub ---- TAP Bridge ---- Real Internet
    |                |
Minecraft / NeoForge
```

## WASM Memory Map

The host (Java) and guest (Rust OS) communicate through fixed memory regions in the WASM linear memory:

| Address | Size | Purpose | Direction |
|---------|------|---------|-----------|
| `0x10000` (64 KiB) | Up to ~4 KiB | Input buffer — keyboard input for `on_input()` | Host → WASM |
| `0x11000` (68 KiB) | Up to ~4 KiB | Interrupt data buffer — payload for `on_interrupt()` | Host → WASM |

### WASM Exports (called by host)

| Export | Signature | Description |
|--------|-----------|-------------|
| `main()` | `() -> void` | Called once when terminal opens |
| `on_input(ptr, len)` | `(i32, i32) -> void` | Called per keyboard input |
| `on_interrupt(irq, ptr, len)` | `(i32, i32, i32) -> void` | Called to deliver an interrupt event |

### Host Functions (callable from WASM)

| Function | Signature | Description |
|----------|-----------|-------------|
| `terminal_write` | `(ptr, len) -> i32` | Write text to terminal |
| `terminal_clear` | `() -> void` | Clear screen |
| `terminal_set_cursor` | `(x, y) -> void` | Move cursor |
| `terminal_get_width` | `() -> i32` | Returns 80 |
| `terminal_get_height` | `() -> i32` | Returns 24 |
| `sleep_ms` | `(ms) -> void` | Sleep (Rust side chunks for interrupt delivery) |
| `redstone_set_output` | `(side, power) -> i32` | Set redstone output (0–15) |
| `redstone_get_input` | `(side) -> i32` | Read redstone input (0–15) |
| `redstone_get_all_input` | `(buf_ptr) -> i32` | Read all 6 input sides into buffer |
| `interrupt_poll` | `(buf_ptr, buf_len) -> i32` | Poll next pending interrupt (returns IRQ or -1) |
| `interrupt_poll_len` | `() -> i32` | Get payload length of last polled interrupt |
| `file_read/write/delete/exists/size/list` | various | Virtual filesystem operations |
| `peripheral_list/get_methods/call` | various | CC:Tweaked peripheral access |
| `net_get_mac` | `(buf_ptr) -> i32` | Write 6-byte MAC to buffer, returns 6 |
| `net_tx_frame` | `(buf_ptr, frame_len) -> i32` | Transmit raw ethernet frame |
| `net_rx_frame` | `(buf_ptr, buf_len) -> i32` | Non-blocking receive (returns len or -1) |
| `net_rx_frame_blocking` | `(buf_ptr, buf_len, timeout_ms) -> i32` | Blocking receive with timeout |
| `net_set_promiscuous` | `(enabled) -> i32` | Enable/disable promiscuous mode |

## Getting Started

1. Place a **Terminal** block (found in the Redstone and Functional Blocks creative tabs)
2. Right-click to open the terminal
3. Type `help` to see available commands

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
| `peripherals` | `peripherals` | List connected peripherals |
| `peripherals` | `peripherals <name>` | Show methods on a peripheral |
| `ifconfig` | `ifconfig` | Show network configuration |
| `ifconfig` | `ifconfig set <ip> <mask> <gw> [dns]` | Configure network interface |
| `ifconfig` | `ifconfig vlan <id>` | Set 802.1Q VLAN ID (0-4094) |
| `ifconfig` | `ifconfig vlan off` | Disable VLAN tagging |
| `ping` | `ping <ip> [count]` | Send ICMP echo requests |
| `nslookup` | `nslookup <hostname>` | DNS lookup |
| `httpd` | `httpd <port>` | Start HTTP file server |
| `curl` | `curl [options] <url>` | HTTP client |

**Ctrl+T** will terminate any running program and return to the shell.

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

Three built-in modules are available: `terminal`, `peripheral`, and `net`.

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

#### Interrupts

Programs can register interrupt handlers that fire when hardware events occur. Interrupts are cooperative — they are delivered during `sleep()`, `check_interrupts()`, or between commands.

```python
import terminal

# Register a handler for redstone changes
def on_redstone(data):
    terminal.println(f"Redstone changed! Sides: {data['sides']}")

terminal.on_interrupt(terminal.IRQ_REDSTONE, on_redstone)

# Register a handler for keypresses during execution
def on_key(data):
    terminal.println(f"Key pressed: {data['key']}")

terminal.on_interrupt(terminal.IRQ_KEYBOARD, on_key)

# In a loop, call check_interrupts() or sleep() to deliver events
while True:
    terminal.sleep(0.1)

# Clear a handler when done
terminal.clear_interrupt(terminal.IRQ_REDSTONE)
```

| IRQ Constant | Value | Event |
|-------------|-------|-------|
| `terminal.IRQ_KEYBOARD` | 1 | Key pressed during execution |
| `terminal.IRQ_REDSTONE` | 2 | Redstone input level changed |
| `IRQ_NETWORK` | 3 | Network frame received |

---

### `peripheral` Module

Terminals can interact with adjacent CC:Tweaked peripherals (requires [CC: Tweaked](https://modrinth.com/mod/cc-tweaked) to be installed).

#### Discovering Peripherals

```python
import peripheral

# List all connected peripherals
peripherals = peripheral.list()
for p in peripherals:
    print(f"{p['name']} ({p['type']}) on {p['side']}")

# Get just the names
names = peripheral.get_names()   # ["chat_box_0", "player_detector_0"]

# Find a peripheral by type (returns name or None)
name = peripheral.find("chat_box")

# Check if a specific peripheral exists
peripheral.is_present("chat_box_0")   # True/False

# Get info about a peripheral
info = peripheral.wrap("chat_box_0")  # {'name': '...', 'type': '...', 'side': '...'}
```

Peripherals are named by type with an incrementing index: `chat_box_0`, `chat_box_1`, `player_detector_0`, etc.

#### Getting Methods

```python
methods = peripheral.get_methods("chat_box_0")
for m in methods:
    print(f"  - {m}")
```

#### Calling Methods

Arguments are passed as a JSON string. Results are returned as a JSON string.

```python
# No arguments
result = peripheral.call("chat_box_0", "getName", "[]")

# With arguments
result = peripheral.call("chat_box_0", "sendMessage", '["Hello World!"]')

# Multiple arguments
result = peripheral.call("some_device", "setConfig", '[10, "label", true]')
```

---

### `net` Module

The `net` module provides a full TCP/IP networking stack built entirely inside the WASM OS. Computers can communicate with each other over a virtual Ethernet LAN, and optionally reach the real internet through a TAP bridge.

#### Network Architecture

```
Computer A ──┐
Computer B ──┼── Ethernet Hub (in-memory) ──┬── frames routed by MAC
Computer C ──┘                              └── TAP Bridge ── Linux kernel ── Internet
```

The host exposes only 5 raw ethernet frame primitives. Everything above Layer 2 — ARP, IPv4, ICMP, UDP, TCP, DNS, HTTP — is implemented from scratch in the Rust OS.

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

#### Configuration

```python
import net

# Configure interface (required before any networking)
net.configure("10.0.0.1", "255.255.255.0", "10.0.0.254", "8.8.8.8")
#              IP           Subnet Mask      Gateway        DNS Server

# View configuration
info = net.ifconfig()
# {'mac': '02:00:00:00:00:01', 'ip': '10.0.0.1', 'mask': '255.255.255.0',
#  'gateway': '10.0.0.254', 'dns': '8.8.8.8', 'configured': True}

# Get MAC address
mac = net.get_mac()  # "02:00:00:00:00:01"
```

Or from the shell:
```
/ > ifconfig set 10.0.0.1 255.255.255.0 10.0.0.254 8.8.8.8
Network configured.
/ > ifconfig
MAC:     02:00:00:00:00:01
IP:      10.0.0.1
Mask:    255.255.255.0
Gateway: 10.0.0.254
DNS:     8.8.8.8
VLAN:    none
```

Each computer gets a unique MAC address derived from its UUID. In Minecraft, this is the block entity's persistent ID. In the simulator, it's `02:00:00:00:00:XX` where XX is the instance index.

#### 802.1Q VLANs

The network stack supports IEEE 802.1Q VLAN tagging, allowing logical network segmentation over the same physical cable. Computers on different VLANs cannot communicate even if physically connected.

```
Standard frame:  [dst 6B][src 6B][ethertype 2B][payload...]
802.1Q frame:    [dst 6B][src 6B][0x8100 2B][TCI 2B][ethertype 2B][payload...]
```

The 4-byte VLAN tag contains a 12-bit VLAN ID (0-4094), 3-bit Priority Code Point, and 1-bit Drop Eligible Indicator.

**Shell:**
```
/ > ifconfig vlan 100
VLAN set to 100.

/ > ifconfig vlan off
VLAN tagging disabled.

/ > ifconfig
MAC:     02:00:00:00:00:01
IP:      10.0.0.1
Mask:    255.255.255.0
Gateway: 10.0.0.254
DNS:     8.8.8.8
VLAN:    100
```

**Python:**
```python
import net

net.vlan_set(100)          # Enable VLAN 100
vid = net.vlan_get()       # Returns 100
net.vlan_set(None)         # Disable VLAN tagging
vid = net.vlan_get()       # Returns None

info = net.ifconfig()
print(info['vlan'])        # 100 or None
```

**Behavior:**
- **No VLAN configured** (default): Only accepts untagged frames. Sends untagged frames.
- **VLAN configured**: Only accepts frames tagged with the matching VLAN ID. Sends all frames (ARP, IPv4) with the VLAN tag.
- Computers on different VLANs are fully isolated — ARP resolution will fail, so no IP communication is possible.
- VLAN configuration is persisted in `network.cfg` and restored on reboot.

#### ICMP (Ping)

```python
import net

rtt = net.ping("10.0.0.2")     # Returns RTT in milliseconds
print(f"Round trip: {rtt}ms")   # Round trip: 12ms
```

Shell:
```
/ > ping 10.0.0.2 4
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

### Host Function Interface

The networking stack sits entirely in the Rust OS (WASM). The host (Java mod or simulator) provides only 5 primitives — raw ethernet frame I/O:

| Function | Signature | Description |
|----------|-----------|-------------|
| `net_get_mac` | `(buf_ptr: i32) -> i32` | Write this computer's 6-byte MAC to buffer. Returns 6. |
| `net_tx_frame` | `(buf_ptr: i32, frame_len: i32) -> i32` | Transmit a raw ethernet frame (14–1514 bytes). Returns 0 on success. |
| `net_rx_frame` | `(buf_ptr: i32, buf_len: i32) -> i32` | Non-blocking receive. Returns frame length or -1 if empty. |
| `net_rx_frame_blocking` | `(buf_ptr: i32, buf_len: i32, timeout_ms: i32) -> i32` | Blocking receive with timeout (max 60s). Returns frame length or -1. |
| `net_set_promiscuous` | `(enabled: i32) -> i32` | Enable (1) or disable (0) promiscuous mode. Returns 0 on success. |

Plus `IRQ_NETWORK = 3` interrupt delivered via `on_interrupt()` when a frame arrives.

### Ethernet Hub

The hub is an in-memory Layer 2 switch that routes frames between computers:

- **Unicast:** Delivered to the NIC matching the destination MAC
- **Broadcast (`ff:ff:ff:ff:ff:ff`):** Delivered to all NICs except sender
- **Unknown unicast:** Forwarded to TAP bridge (if attached) for real internet
- **Promiscuous:** NICs in promiscuous mode receive all frames
- **Queue depth:** 64 frames per NIC (oldest dropped on overflow)

In Java (`NetworkHub.java`), the hub is a server-wide singleton initialized on server start. Each `ComputerInstance` registers its NIC on creation and unregisters on close.

In the simulator (`hub.rs`), the hub is shared between all WASM instances via `Arc<EthernetHub>`.

### TAP Bridge — Real Internet Access

The TAP bridge connects the in-game Ethernet hub to a real Linux TAP device, enabling computers to access the actual internet.

```
WASM Computer → net_tx_frame() → Hub → TAP Writer Thread → /dev/net/tun → Linux Kernel → NAT → Internet
Internet → Linux Kernel → /dev/net/tun → TAP Reader Thread → Hub → IRQ_NETWORK → net_rx_frame() → WASM Computer
```

**Setup (simulator):**
```bash
# 1. Create TAP device and NAT rules (as root)
sudo scripts/setup-tap.sh tap0

# 2. Run simulator with TAP
cargo run --release -- --tap tap0 --auto-net

# 3. From the terminal:
/ > ping 8.8.8.8
Reply from 8.8.8.8: time=30ms seq=0
/ > nslookup example.com
Address: 93.184.216.34
/ > curl http://example.com/
<!doctype html>...

# 4. Teardown when done
sudo scripts/teardown-tap.sh tap0
```

**Setup (Minecraft server):** Configure in the mod's config file. Requires the TAP device to be pre-created with `scripts/setup-tap.sh`. The Java mod uses a Python helper process to open the TAP device (avoids JNI).

**Network topology with TAP:**
- Computers on the same server share a virtual LAN (10.0.0.0/24)
- Each computer gets an IP like 10.0.0.1, 10.0.0.2, etc.
- The TAP device acts as the gateway at 10.0.0.254
- Linux iptables provides NAT for outbound traffic
- Computers can reach each other directly AND access the real internet

### OS Networking Stack — Implementation Details

All networking code lives in `operating-system/rust/src/net/`:

```
net/
  mod.rs          — NetStack singleton: poll_rx, poll_timers, send_ipv4, high-level API
  types.rs        — MacAddr, Ipv4Addr, SocketAddr, NetError
  checksum.rs     — RFC 1071 internet checksum + TCP/UDP pseudo-header checksum
  eth.rs          — Ethernet frame parse/serialize (14-byte header), host function wrappers
  arp.rs          — ARP table (32 entries, 5-min TTL), request/reply, gratuitous ARP
  ipv4.rs         — IPv4 header (20 bytes), checksum, RoutingTable (subnet + gateway)
  icmp.rs         — ICMP echo request/reply
  udp.rs          — UdpSocketTable (8 sockets), bind/send/recv with ring buffers
  tcp.rs          — TcpConnectionTable (8 connections), full state machine, retransmission
  dns.rs          — DNS query builder, response parser, name encoding/compression
  http.rs         — HTTP/1.0 request parser, response builder, client, URL parser
```

**Packet reception flow:**
```
IRQ_NETWORK → NetStack::poll_rx() → net_rx_frame()
  → EthHeader::parse() → dispatch by ethertype
    → ARP: update table, send reply if for us
    → IPv4: Ipv4Header::parse() → dispatch by protocol
      → ICMP: auto-reply to echo requests, update ping state
      → UDP: deliver to bound socket's rx ring buffer
      → TCP: process_segment() state machine → update connection state
```

**Packet transmission flow:**
```
Application calls (e.g., tcp_send, udp_send, ping)
  → NetStack::send_ipv4(dst_ip, protocol, payload)
    → RoutingTable::next_hop() — same subnet or gateway?
    → ARP lookup for MAC — if miss, send ARP request, return WouldBlock
    → Build IPv4 header (checksum computed on serialize)
    → Build Ethernet frame (dst_mac, src_mac, 0x0800)
    → net_tx_frame() — hand to host
```

**Timer-driven events:** `NetStack::poll_timers()` is called periodically (during `sleep()` chunks and network operations) and drives:
- TCP retransmission (exponential backoff)
- TCP TIME_WAIT expiry
- ARP cache expiry

**Memory budget (static allocation in WASM):**

| Component | Size |
|-----------|------|
| RX/TX frame buffers | 3 KB |
| ARP table (32 entries) | ~320 B |
| UDP sockets (8 x 2KB) | 16 KB |
| TCP connections (8 x 8KB) | 66 KB |
| Payload scratch buffer | 1.5 KB |
| **Total** | **~87 KB** |

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

### Chat Box

```python
import terminal
import peripheral

chat = peripheral.find("chat_box")
if not chat:
    terminal.println("No chat box found! Place one next to the terminal.")
    exit()

peripheral.call(chat, "sendMessage", '["Hello from the terminal!"]')
terminal.println("Message sent!")
```

### Player Detector

```python
import terminal
import peripheral

detector = peripheral.find("player_detector")
if not detector:
    terminal.println("No player detector found!")
    exit()

result = peripheral.call(detector, "getOnlinePlayers", "[]")
terminal.println(f"Online players: {result}")
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

### Peripheral Wrapper Class

For cleaner code when working with peripherals frequently:

```python
import peripheral

class Device:
    def __init__(self, name):
        self.name = name

    def call(self, method, args="[]"):
        return peripheral.call(self.name, method, args)

    def methods(self):
        return peripheral.get_methods(self.name)

# Usage
chat = Device("chat_box_0")
chat.call("sendMessage", '["Automated message"]')
```

### Redstone Input Monitor

Read redstone input and react to changes using interrupts:

```python
import terminal

def on_redstone(data):
    sides = data['sides']
    terminal.set_cursor(0, 2)
    terminal.write(f"Input levels: {sides}   ")
    # Mirror back input to front output
    terminal.set_redstone(terminal.FRONT, sides[3])  # BACK input -> FRONT output

terminal.on_interrupt(terminal.IRQ_REDSTONE, on_redstone)

terminal.println("Redstone monitor running. Ctrl+T to stop.")
terminal.println("Apply redstone to any side to see levels.")

while True:
    terminal.sleep(1.0)
```

### Keyboard Event Logger

Log keypresses during execution:

```python
import terminal

keys = []

def on_key(data):
    keys.append(data['key'])
    terminal.set_cursor(0, 2)
    terminal.write(f"Last key: {repr(data['key'])}  Total: {len(keys)}   ")

terminal.on_interrupt(terminal.IRQ_KEYBOARD, on_key)

terminal.println("Press keys while running. Ctrl+T to stop.")
while True:
    terminal.sleep(0.1)
```

---

## Mod Integration API

EvansComputerMod is designed for other mods to extend. You can expose Java methods to Python with a few annotations — no WASM knowledge required. You can also embed a full computer into your own blocks, entities, or items.

### Quick Start: Exposing Functions to Python

**1. Add EvansComputerMod as a dependency** in your `build.gradle`:

```groovy
dependencies {
    implementation 'com.example:evanscomputermod:1.0.0'
}
```

**2. Create an annotated module class:**

```java
import com.example.evanscomputermod.api.*;

@ComputerModule(value = "golem", description = "Golem control API")
public class GolemAPI {

    @ComputerFunction(description = "Summon a golem of the given type")
    public boolean summon(ComputerContext ctx, String type) {
        // ctx gives you computerId, level, position, server
        // ... your mod logic here ...
        return true;
    }

    @ComputerFunction(description = "Get the health of a golem")
    public int getHealth(String golemId) {
        // ComputerContext is optional — omit it if you don't need world access
        return 20;
    }

    @ComputerFunction(description = "Detonate a golem", mainThread = true)
    public void detonate(ComputerContext ctx, String golemId) {
        // mainThread = true ensures this runs on the server tick thread
        // (required for any operation that modifies the world)
    }
}
```

**3. Register during the setup event:**

```java
import com.example.evanscomputermod.api.ComputerModuleRegistry;
import com.example.evanscomputermod.api.RegisterComputerModulesEvent;
import net.neoforged.bus.api.SubscribeEvent;

public class MyMod {
    @SubscribeEvent
    public void onRegisterModules(RegisterComputerModulesEvent event) {
        ComputerModuleRegistry.register(new GolemAPI());
    }
}
```

**4. That's it.** Python users can now do:

```python
import golem

golem.summon("iron")          # calls GolemAPI.summon()
health = golem.get_health("golem_1")  # calls GolemAPI.getHealth()
golem.detonate("golem_1")     # runs on main thread
```

Visual programming blocks are also auto-generated — one block per function, with typed input/output ports.

### Annotation Reference

#### `@ComputerModule(value, description)`

| Attribute | Type | Required | Description |
|-----------|------|----------|-------------|
| `value` | `String` | Yes | Python module name (what users `import`) |
| `description` | `String` | No | Description for visual block category |

#### `@ComputerFunction(value, description, mainThread)`

| Attribute | Type | Default | Description |
|-----------|------|---------|-------------|
| `value` | `String` | `""` | Override Python function name (default: camelCase converted to snake_case) |
| `description` | `String` | `""` | Description for visual blocks and help text |
| `mainThread` | `boolean` | `false` | Execute on the server main thread (required for world modifications) |

#### `ComputerContext`

Injected as the first parameter of your method if present. Not visible to Python callers.

| Field | Type | Description |
|-------|------|-------------|
| `getComputerId()` | `UUID` | Unique persistent ID of the computer |
| `getPosition()` | `BlockPos` | World position (null if headless) |
| `getLevel()` | `Level` | Minecraft level/world (null if headless) |
| `getServer()` | `MinecraftServer` | Server instance |

#### Supported Types

| Java Type | Python Type | Binary Tag |
|-----------|-------------|------------|
| `String` | `str` | `0x01` |
| `int` / `Integer` | `int` | `0x02` |
| `long` / `Long` | `int` | `0x03` |
| `float` / `Float` / `double` / `Double` | `float` | `0x04` |
| `boolean` / `Boolean` | `bool` | `0x05` |
| `void` | `None` | `0x00` |

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
| `getWorldAccess()` | `IWorldAccess` | No (null = no peripherals) | Position for peripheral scanning |
| `getVisualProgramming()` | `IVisualProgramming` | No (null = disabled) | Visual editor support |

---

### How It Works: Technical Architecture

Understanding the internals is not required to use the API, but this section explains how annotations become Python functions.

#### The WASM Recompilation Problem

The computer runs a Rust OS compiled to WebAssembly. Host functions (Java methods callable from WASM) must be declared as `extern "C"` in Rust **at compile time**. Third-party mods cannot modify the WASM binary.

The solution: a single generic bridge function `module_call` is compiled into the WASM binary once. All third-party module calls route through it. The bridge uses a binary protocol instead of JSON for efficiency.

#### The Full Pipeline

```
Python: golem.summon("iron")
  |
  | (1) Python bootstrap auto-generated this function at startup.
  |     It calls: _modules.call("golem", "summon", "iron")
  v
Rust (python.rs): serialize_args_binary()
  |
  | (2) Converts Python objects to binary: [0x01 arg_count] [0x01 tag=string] [0x04 len] [iron]
  |     No JSON escaping, no string building — just type tag + raw bytes.
  v
Rust (modules.rs): module_call() extern "C"
  |
  | (3) Writes module name, method name, and binary args into WASM linear memory.
  |     Calls the host function (crosses WASM→Java boundary via Wasmtime).
  v
Java (ComputerInstance): hostModuleCall()
  |
  | (4) Reads module/method names as strings from WASM memory.
  |     Reads args as raw bytes (no string conversion).
  |     Delegates to ModuleMethodInvoker.
  v
Java (ModuleMethodInvoker): parseBinaryArgs()
  |
  | (5) Reads type tags from binary buffer.
  |     Constructs typed Java objects directly (Integer, String, Boolean, etc.)
  |     Uses ParameterInfo from @ComputerFunction annotation for type coercion.
  |     Injects ComputerContext if method expects it.
  v
Java: GolemAPI.summon(ctx, "iron")
  |
  | (6) Your mod code runs. Returns a boolean.
  v
Java (ModuleMethodInvoker): serializeResult()
  |
  | (7) Writes: [0x00 status=ok] [0x05 tag=bool] [0x01 value=true]
  |     3 bytes total. No JSON object construction.
  v
WASM memory → Rust: parse_binary_result()
  |
  | (8) Reads status byte, type tag, payload.
  |     Returns BinaryValue::Bool(true). No string parsing.
  v
Rust (python.rs): binary_value_to_pyobj()
  |
  | (9) Converts BinaryValue to Python bool directly.
  v
Python: True
```

#### Binary Wire Format

Arguments and results use a compact type-tagged binary encoding:

```
Arguments: [u8 arg_count] ([u8 type_tag] [payload])*
Result:    [u8 status] [u8 type_tag] [payload]

Type tags:
  0x00 = null
  0x01 = string:  [u32 LE length] [UTF-8 bytes]
  0x02 = i32:     [4 bytes LE]
  0x03 = i64:     [8 bytes LE]
  0x04 = f64:     [8 bytes LE]
  0x05 = bool:    [1 byte, 0 or 1]

Status: 0x00 = success, 0x01 = error (followed by string message)
```

Example: `golem.summon("iron")` produces 10 bytes of args (`01 01 04000000 69726F6E`) and 3 bytes of result (`00 05 01`). Compare this to the equivalent JSON which would be `["iron"]` (8 bytes) and `{"ok":true,"result":true}` (24 bytes) — plus the overhead of parsing both.

#### Auto-Generated Python Modules

At interpreter startup, `python_bootstrap.py` calls `_modules.get_metadata()` which returns JSON metadata describing all registered modules and their functions. The bootstrap creates a Python `ModuleType` for each module and populates it with wrapper functions that delegate to `_modules.call()`. This happens once, before any user code runs.

#### Auto-Generated Visual Programming Blocks

`VisualBlockRegistry.generateBlocksFromModules()` iterates `ComputerModuleRegistry` at load time and creates visual block definitions for each registered function. Each block gets typed input ports (from `ParameterInfo`), flow ports, and a code template that generates the correct Python call. A deterministic color is assigned based on the module name.

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

## Installation

1. Install [NeoForge](https://neoforged.net/) for Minecraft 1.21.1
2. Download the mod JAR from [Releases](https://github.com/ewitulsk/EvanModCursor/releases)
3. Place it in your `mods/` folder
4. (Optional) Install [CC: Tweaked](https://modrinth.com/mod/cc-tweaked) for peripheral support

## License

MIT
