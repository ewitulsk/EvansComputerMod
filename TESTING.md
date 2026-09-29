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
- The test copies the kernel and the programs it needs (`echo`, `sleep`, `ssh`, `sshd`) from `rust/target` into a temporary `wasm-bin`.
- It *skips* if they aren't built. The runner stages them first, and treats a skipped test as a failure.

### 4. GameTests (in-world)

```powershell
scripts\Test.ps1 -Area network-ingame -GameTests ecm_network
```

- **Registration (26.1).** Tests are registered through `RegisterGameTestsEvent` into the registry-based framework (26.1 has no `@GameTest` annotations). They run on the generated empty structure `evanscomputermod:gametest_empty`, which `scripts/gen-gametest-structure.py` writes. Setups are built in code.
- **One namespace per feature area.** Namespaces can be comma-separated in one launch.
  - `ecm_network`: ping and SSH between two cabled terminals, plus a no-cable control.
- **Pass markers.** Each passing test logs `ECM_<AREA>_TEST_PASS <case>`, e.g. `ECM_NETWORK_TEST_PASS ssh_between_cabled_terminals`. The runner requires exactly one marker per registered test and NeoForge's `All N required tests passed`.
- **Why wall-clock time, not ticks.** Computers run in real time on their own worker threads, but the GameTest server ticks as fast as it can: 1200 ticks go by in about 3 s. So each test is bounded by a **60 s wall-clock limit** inside its step script, and the tick limit is only a backstop. The reason is written in `NetworkGameTests.onRegisterTests`. A test that stalls fails with the step it was stuck on and both screens dumped.
- **Control cases.** Every area includes one, e.g. `no_cable_no_ping`: the same setup without the cable must *not* get a reply.
- **Isolation.** Test worlds live in `runs/gametest-<timestamp>/`, inside the project and gitignored. Never point a run at a real profile or save.
- **Version.** 26.1 only, for the same Sable reason as JUnit.

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
| `ssh-client`, `sshd`, `ecm-ssh-*`, session syscalls | `-JUnit KernelHostIntegrationTest -GameTests ecm_network` |
| Other WASI programs (`rust/wasm-programs/*`) | the scenario or JUnit test that uses the program; add one if none does |
| Simulator (`rust/simulator/**`) | `-Scenarios <filter>` for the affected scenarios, plus `cargo test -p terminal-simulator` for its unit tests |
| Rendering, client screens, input | not covered by automation yet: test manually in a client, and say so in your report |

If you're not sure whether something is affected, look at what calls the changed code, not at the whole suite.

---

## Hard rules (from the shared guide)

- Compiling, or a run where nothing executed, is **not** a pass. Report exactly what ran and what passed, with the receipt path.
- Don't weaken a test to make it pass: no loosened limits, no skipped steps, no removed control cases. If a limit really is wrong, say why before changing it.
- Never edit a `FAIL` receipt; failed artifacts are kept.
- Label tests you wrote but didn't run as "written, not run".
- Test worlds and files stay inside the project.
- Report honestly which areas you deliberately didn't test and why.
