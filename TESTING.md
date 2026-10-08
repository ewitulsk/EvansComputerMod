# Testing EvansComputerMod

This is the mod-specific version of the shared guide, [`../ModTesting.md`](../ModTesting.md). Its three rules apply unchanged:

1. **Implement a large chunk first, then test it.** Compiling as you go is fine (`cargo build`, `./gradlew compileJava`).
2. **Run only the tests that cover what you changed.** Use the [map](#what-to-run-for-what-you-changed) below. There is no "run everything" step.
3. **Keep each test short:** one minute is the ceiling. For this mod, the minute is measured on the **wall clock** for anything that runs a real computer (see [GameTests](#4-gametests-in-world)).

All runs go through one runner, which writes a receipt:

```powershell
scripts\Test.ps1 -Area <name> [-Rust <crates>] [-JUnit <classes>] [-GameTests <namespaces>] [-Scenarios <filters>]
```

It writes `artifacts/<area>-yyyyMMdd-HHmmss/result.json` plus one log per step. The status is `DIAGNOSTIC_PASS` for a passing targeted run and `FAIL` otherwise. Pass/fail comes from what actually ran (test counts, reports, pass markers), never from exit codes alone, and a run where nothing ran is a `FAIL`. Before any Java or in-world step, the runner rebuilds and stages the kernel and programs (`scripts/stage-wasm.sh`); pass `-NoStage` to skip that.

---

## Test layers (cheapest first)

Most of this mod is Rust compiled to WASM, so the cheapest layers are plain `cargo test` runs on the host. Use the cheapest layer that can catch the bug.

| # | Layer | Where | Use it for | Cost |
|---|---|---|---|---|
| 1 | **Rust crate tests** | `rust/crates/ecm-net`, `rust/crates/ecm-bridge`, `rust/operating-system/rust` (`terminal-os`) | Protocols, parsers, TCP under loss, switch STP/LACP/LLDP/VLAN, kernel shell/jobs/socket IPC, the network dispatcher (TCP through a switch, loops, trunks), fuzzing | Seconds |
| 2 | **ABI check** | `scripts/check-abi.py` | The built kernel's imports/exports vs. [`abi/host-abi.toml`](abi/host-abi.toml) | Instant |
| 3 | **JUnit with the real kernel** | `src/test/java`, e.g. `KernelHostIntegrationTest` | The Java host (`ComputerInstance`, IPC bridge, process manager) running the real `terminal_os.wasm` on Chicory: boot, programs, Ctrl+T, SSH over loopback. No world. | ~1 min (Gradle) |
| 4 | **GameTests (in-world)** | `src/main/java/.../testing/*GameTests.java` | Real Terminal and cable blocks in a real world: cabling, the NIC/face mapping, `NetworkHub`/`CableNetworkManager`, two computers talking (ping, SSH) | ~1 min (server start) |
| 5 | **Simulator scenarios** | `rust/simulator/scenarios/*.toml` | Many computers in a topology file with a virtual clock and fault injection: switches, STP rings, LAGs, lossy links | Seconds–minute |

The client layer from the shared guide (hidden client, replayed input) isn't used yet. Only rendering, and what a player sees on the terminal screen, would need it.

**Keep code testable at the cheap layers.** Protocol and kernel logic lives in plain Rust with no host calls: `ecm-net` and `ecm-bridge` are sans-IO, and the kernel only reaches the host through `hal.rs` and the `Nics` trait. Put new logic there and keep the Java/Minecraft glue thin.

### 1. Rust crate tests

```powershell
scripts\Test.ps1 -Area net    -Rust ecm-net
scripts\Test.ps1 -Area switch -Rust ecm-bridge
scripts\Test.ps1 -Area kernel -Rust terminal-os
```

The in-process harnesses run on virtual time, so they are fast and deterministic:

- `ecm-net/tests` wires stacks back to back.
- `ecm-bridge/tests` is a multi-bridge netsim.
- `terminal-os/src/net/net_tests.rs` drives the real kernel dispatcher.

Kernel tests that touch the shared framebuffer statics take `hal::test_lock()`.

### 2. ABI check

```bash
python3 scripts/check-abi.py        # after building the kernel
```

Run it whenever you change kernel imports or exports, `hal.rs`, or the host function table in `ComputerInstance`. The Java host also refuses to load a mismatched kernel.

### 3. JUnit with the real kernel

```powershell
scripts\Test.ps1 -Area host -JUnit KernelHostIntegrationTest
```

- This runs on **MC 26.1** (`:26.1:test`). JUnit runs with NeoForge on the classpath (`neoForge.unitTest`), and the 1.21.1 variant can't load the mod there because Sable is compile-only.
- The test copies the kernel and the programs it needs (`echo` and `sleep`, plus `ssh`, `sshd`, `controllertest`, `beep`, `python` and `gba` when they are built) from `rust/target` into a temporary `wasm-bin`.
- It *skips* if they aren't built. The runner stages them first, and treats a skipped test as a failure.

### 4. GameTests (in-world)

```powershell
scripts\Test.ps1 -Area network-ingame -GameTests ecm_network
```

- **Registration.** 1.21.1 (the default, `-McVersion 1.21.1`) uses the annotation API: `@GameTestHolder(<namespace>)` classes in `testing/v1211/`, one `batch` per test so they run one after another. 1.21.1 looks a structure up under the holder namespace, so `scripts/gen-gametest-structure.py` writes a copy per namespace into `src/main/resources-mc1.21.1/`. 26.1 (`-McVersion 26.1`) registers through `RegisterGameTestsEvent` into the registry-based framework (`NetworkGameTests`, `SwitchGameTests`). Setups are built in code on empty structures either way.
- **One namespace per feature area.** Namespaces can be comma-separated in one launch.
  - `ecm_router` (1.21.1): forwarding/NAT/DHCP with a disabled-forwarding control, headless unload/reattach of the same live kernel, BGP traffic rerouting after a logical fiber cut with an isolation control, actual fiber block removal/replacement, all twenty village infrastructure nodes booting without terrain and serving HTTP across villages, and a jigsaw village with fifteen distinct provisioned computer UUIDs. Each case is bounded below a minute.
  - `ecm_network`: ping and SSH between two cabled terminals, plus a no-cable control.
  - `ecm_sync` (1.21.1): does a client's terminal screen match the server's? `ClientMirror` plays a client with the real delta packets and client apply code, plus block-entity updates, and every comparison is written to `screenshots/` as a PNG (server vs client, differing rows red; the runner copies them to `artifacts/<area>-<ts>/screenshots/`). It reproduces the "output printed twice until the GUI is reopened" bug: see `docs/images/display-sync-before-fix.png` / `-after-fix.png`.
  - `ecm_periph` (1.21.1): module bays (install, eject, drop keeps settings), block peripherals next to a computer, and the Redstone Link module against real Create links in both directions, plus one end-to-end run of the `peripherals` command and a Python program on a booted computer. The runner puts Create (`libs/create-1.21.1-*.jar`, fetched by `scripts/fetch-libs.sh`) into the run's `mods/`. Create's link network is level-wide and finished tests' blocks stay loaded, so each test uses its own frequency pair.
  - `ecm_sensor` (1.21.1): Sensor Wire placed through the item's click handling (routing included) from a Wired Sensor Module's bay connector to a lidar, the module's sensor discovery, naming and mounts, lidar ranges against a block and an entity and points in the computer frame, a second sensor on a junction off the first wire, the wire moving into a Sable structure (and a scan from the structure hitting its own computer), one Python program using the `sensors` module on a booted computer, and the `lidar_room` scenario (`testing/scenario/SensorScenarios.java`, spawnable with `/ecm scenario spawn lidar_room`), whose drawn map is written to the log. Uses the larger `gametest_sensor` structure; the runner loads Sable, which the move test needs.
  - `ecm_screen` (1.21.1): a program owning a Screen cluster at its own resolution (`gba` running a test ROM at 240x160) keeps the screen, powered and updating, through a block update beside the terminal.
  - `ecm_switch`: the switching debug scenarios (`testing/scenario/SwitchScenarios.java`): one switch with three hosts, VLAN isolation, a trunk between two switches with an SVI and LLDP, an STP loop with a cable cut, and an LACP LAG with a cable cut. Each scenario runs in its own batch, one after another; the namespace takes about 1 minute. They use the larger `gametest_switch` structure.
- **The same scenarios in a normal world.** `/ecm scenario spawn <name> [auto|manual|fast]` (op only) builds a scenario 3 blocks south of you and types its script into the terminals, reporting each step in chat. `manual` builds and boots only, then prints the commands. `/ecm scenario commands <name>` prints the script, `rerun` rebuilds in place, `clear` removes everything spawned. Spawning clears the layout's box to air.
- **Pass markers.** Each passing test logs `ECM_<AREA>_TEST_PASS <case>`, e.g. `ECM_NETWORK_TEST_PASS ssh_between_cabled_terminals`. The runner requires exactly one marker per registered test and NeoForge's `All N required tests passed`.
- **Why wall-clock time, not ticks.** Computers run in real time on their own worker threads, but the GameTest server ticks as fast as it can: 1200 ticks go by in about 3 s. So each test is bounded by a **60 s wall-clock limit** inside its step script, and the tick limit is only a backstop. The reason is written in `NetworkGameTests.onRegisterTests`. Keep the tick backstop far above a minute of unthrottled ticks (`SwitchGameTests` uses `Integer.MAX_VALUE / 2`); otherwise it fires first and hides the real failure. Also, throwing inside `succeedWhen` only means “not yet”, so end a test early with a sequence's `thenFail`, as `SwitchGameTests` does. A test that stalls fails with the step it was stuck on and both screens dumped.
- **Control cases.** Every area includes one, e.g. `no_cable_no_ping`: the same setup without the cable must *not* get a reply.
- **Isolation.** Test worlds live in `runs/gametest-<timestamp>/`, inside the project and gitignored. Never point a run at a real profile or save.
- **Version.** 1.21.1: `ecm_switch`, `ecm_sync`, `ecm_periph`, `ecm_sensor`, `ecm_screen`. 26.1: `ecm_network`, `ecm_switch`. On 1.21.1 the mod implements Sable interfaces, so the runner copies `libs/sable-neoforge-1.21.1-*.jar` into the run's `mods/` folder (Sable 2.0.5 needs NeoForge >= 21.1.228, which is what 1.21.1 builds against).

### 5. Simulator scenarios

```powershell
scripts\Test.ps1 -Area switch-sim -Scenarios switch_
```

- Scenario files and their format are documented in [`rust/simulator/README.md`](rust/simulator/README.md). `-Scenarios` takes cargo test-name filters for `terminal-simulator`.
- Use this layer for anything needing more than two computers or controlled faults: switched networks, STP rings, LAG failover, packet loss.
- Scenarios run on a **virtual clock**, so their time limits are in simulated seconds and they still finish in real seconds.

---

## What to run for what you changed

| You changed | Run |
|---|---|
| `rust/crates/ecm-net/**` (stack, TCP, ARP, DNS, parsers) | `-Rust ecm-net,terminal-os` (the kernel's IPC and dispatcher tests use the stack) |
| `rust/crates/ecm-bridge/**` (switching, STP/LACP/LLDP, CLI) | `-Rust ecm-bridge,terminal-os`; `-Scenarios switch_` for multi-switch behaviour |
| Kernel `net/` (dispatcher, socket IPC, netlink, config) | `-Rust terminal-os`; `-GameTests ecm_network` if it affects real traffic |
| Kernel shell, jobs, line editor, sessions (`shell.rs`, `jobs.rs`, `lineedit.rs`, `sessions.rs`, `kernel.rs`) | `-Rust terminal-os -JUnit KernelHostIntegrationTest` |
| `hal.rs`, kernel exports, `abi/host-abi.toml` | `python3 scripts/check-abi.py`; `-JUnit KernelHostIntegrationTest` |
| `ComputerInstance`, `wasi/*` (worker loop, IPC bridge, process manager, WASI functions) | `-JUnit KernelHostIntegrationTest`; `-GameTests ecm_network` for networking paths |
| `NetworkHub`, `CableNetworkManager`, cable/terminal/interface blocks, NIC discovery | `-GameTests ecm_network` |
| Switch service (`switch_svc.rs`), or bridge changes that should hold on real blocks | `-GameTests ecm_switch` |
| Terminal screen sync (`FramebufferDiffTracker`, `ClientSyncState`, `TerminalDeltaPacket`, `DeltaApplier`, `TerminalBlockEntity` sync / update tag) | `-GameTests ecm_sync` |
| Peripherals and modules (`api/peripheral`, `api/module`, `computer/peripheral`, `module/*`, `compat/create/*`, terminal bays / `useItemOn`, `peripheral` Python module, `ecm_host_abi::peripheral`) | `-Rust ecm-host-abi -GameTests ecm_periph` (needs Create, which the runner loads; `-NoCreate` checks the mod still loads without it) |
| Sensors and wires (`sensor/**`, `TerminalWireHost`, the `sensors` Python module, `WIRED_SENSOR` bay visual) | `-GameTests ecm_sensor` (add `ecm_periph` if module bays changed) |
| `ssh-client`, `sshd`, `ecm-ssh-*`, session syscalls | `-JUnit KernelHostIntegrationTest -GameTests ecm_network` |
| Other WASI programs (`rust/wasm-programs/*`) | the scenario or JUnit test that uses the program; add one if none does |
| Display devices (`computer/display/*`, `gfx_*` WASI functions, `ecm_host_abi::gfx_child`) | `-JUnit DisplayDeviceTest,KernelHostIntegrationTest`; Screen clusters (`rescanScreenCluster`, `ScreenClusterDiscovery`): `-GameTests ecm_screen` |
| Wi-Fi client (`radio/wifi/**` module and low MAC, `wifi_*` host functions, kernel `net/wifi.rs`, `iw`, `wpa_supplicant`, `wpa_cli`, `tcpdump -i wlan0`) | `-Rust terminal-os,ecm-wifi,ecm-host-abi,wpa_supplicant,wpa_cli,iw -JUnit LowMacTest,KernelHostIntegrationTest -GameTests ecm_radio` (scenarios `wifi_monitor`, `wifi_wpa2_ping`; the controller-mode test) and `python scripts/check-abi.py` |
| Wireless controller (`controller/*`, `ecm_host_abi::gamepad`, `controller` Python module) | `-Rust ecm-host-abi -JUnit WirelessControllerHubTest,KernelHostIntegrationTest`; the item, binding screen and key capture are client code: test manually |
| Speaker (`speaker/*`, `DeviceFd`, `/dev/audio*`, `ecm-audio`, `audio` Python module) | `-Rust ecm-audio -JUnit SpeakerAudioTest,KernelHostIntegrationTest`; what players hear is client code: test manually |
| `gba` or `rust/third_party/rustboyadvance-ng` | `cargo test --release -p gba -p rustboyadvance-core` (test ROMs, saves, PSG); `-JUnit KernelHostIntegrationTest` for the program on the host |
| `ChicoryRuntime`, wasmtime sidecar, WASI clocks | `-JUnit WasiClockTest,KernelHostIntegrationTest` |
| Simulator (`rust/simulator/**`) | `-Scenarios <filter>` for the affected scenarios, plus `cargo test -p terminal-simulator` for its unit tests |
| New router laboratories | `-GameTests ecm_router_scenarios`; these execute the same definitions as `/ecm scenario spawn router_*` |
| Fiber models and Tech Village teleport | `-ClientChecks` runs a fresh normal-world server and hidden Minecraft 1.21.1 client; verifies natural generation, neighbor updates, baked models and four paired screenshot cases |
| Radio API, medium, link cache, propagation (`radio/api`, `radio/medium/**`, `radio/phys/**`) | `-JUnit BasicRadioMediumTest,WorldRadioMediumTest,PathTracerTest -McVersion 26.1` (and the `radio.phys` tests); `-GameTests ecm_radio` |
| Antennas, conductors, solver (`radio/antenna/**`, `radio/conductor/**`) | `-JUnit AntennaGraphAnalysisTest -McVersion 26.1` (+ `radio.antenna.solver` tests); `-GameTests ecm_radio` |
| Wi-Fi (`radio/wifi80211/**`, `radio/wifi/**`, `ecm-wifi`, `wpa_supplicant`/`iw`/`wpa_cli`, kernel `wlan0`) | `-Rust ecm-wifi,terminal-os -JUnit Wpa2CryptoVectorsTest,FrameCodecTest,AccessPointCoreTest -McVersion 26.1`; `-GameTests ecm_radio` |
| SDR, DSP, handheld, `radio` Python, SDR programs, `radio0` (`radio/sdr/**`, `radio/handheld/**`, `ecm-dsp`, `ecm-radio`) | `-Rust ecm-dsp,ecm-radio,python -JUnit SdrRadioTest,HandheldDemodTest -McVersion 26.1`; `-GameTests ecm_radio` |
| Packet sockets, DHCP (`net/packet.rs`, `dhcpd`, `dhclient`) | `-Rust terminal-os,ecm-net,ecm-router -Scenarios dhcp`; `-GameTests ecm_radio` (`dhcp_lan`) |
| Microwave, amplifiers, hazards, Sable/Aeronautics radio | `-GameTests ecm_radio`; add `-Aeronautics` to load Create Aeronautics (needs `libs/optional/create-aeronautics-bundled-1.21.1-*.jar`) |
| Radio block/item models | `-ClientChecks -ClientSuite radio` (every registered radio block and item: baked models, sprites, paired screenshots) |
| Other rendering, client screens, input | add a bounded scripted check to the hidden-client runner; use KeyMapping replay for player input, never desktop automation |

If you're not sure whether something is affected, look at what calls the changed code, not at the whole suite.

---

## Hard rules (from the shared guide)

- Compiling, or a run where nothing executed, is **not** a pass. Report exactly what ran and what passed, with the receipt path.
- Don't weaken a test to make it pass: no loosened limits, no skipped steps, no removed control cases. If a limit really is wrong, say why before changing it.
- Never edit a `FAIL` receipt; failed artifacts are kept.
- Label tests you wrote but didn't run as "written, not run".
- Test worlds and files stay inside the project.
- Report honestly which areas you deliberately didn't test and why.

## Real Minecraft render checks

```powershell
scripts/Test.ps1 -Area tech-models -ClientChecks -NoStage
scripts/Test.ps1 -Area radio-render -ClientChecks -ClientSuite radio -NoStage
scripts/Test.ps1 -Area router-labs -GameTests ecm_router_scenarios -NoStage
```

The client check creates isolated server/client directories under `runs/`, uses
a free loopback port and offline test profile, disables the early loading window,
and launches hidden processes. Opt-in mixins suppress window focus, monitor
changes, mouse capture and desktop error dialogs. No user game profile or world
is opened. The server generates village 3 naturally in a fresh normal world,
asserts its 15 distinct computers, and builds an asset display. The hidden client
checks nonmissing baked geometry and captures connected, disconnected, repaired
fiber and the actual village teleport. Screenshots must contain varied pixels;
every case needs both server and client pass markers. Inspect the screenshots
before reporting visual quality. Startup is bounded separately; the ready-world
scenario has a 55-second limit. Receipts retain logs and screenshots together.

Every major feature must also add a usable scenario with a walkthrough and
meaningful positive/negative controls, as required by `AGENTS.md`.
