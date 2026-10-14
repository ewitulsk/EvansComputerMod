# EvansComputerMod development

Follow the targeted testing workflow in `TESTING.md` and `../ModTesting.md`.
Build a coherent change before running its tests, retain failure receipts, and
report executed counts. Tests and test worlds stay inside this repository.

Every major gameplay feature must include usable `/ecm scenario spawn` scenarios.
Register them in the existing scenario command, provide `auto`, `manual`, and
`fast` modes where applicable, and make `/ecm scenario commands` explain the
setup, commands, expected results, and meaningful failure/control cases. Reuse
the same definitions in targeted GameTests rather than maintaining separate
implementations. A feature is unfinished if its scenarios are absent, silently
skip its important behavior, or were written without being run.

Build devices from primitives. This mod recreates real-world devices out of
low-level primitives instead of special-purpose blocks: a router is a computer
running a real routing program, not a "router block". New features follow the
same path:
1. Build on the primitives that already exist (computers and their programs,
   cables and network interfaces, the radio medium, antennas and SDRs,
   peripherals and modules, sockets, `tun`, the UI toolkit).
2. If a needed primitive is missing, implement that primitive first, as a
   general reusable building block with its own tests, and then build the
   feature on top of it.
3. The behaviour of a device (its protocol, policy, management and UI) lives
   in software running on computers, written in Rust/Python against the ABI.
   Java provides hardware: physics, signals, blocks, items, and interfaces to
   them. Do not hide a whole device's logic inside a block entity.
A block that bundles a complete device (for example a cell tower that runs its
whole protocol internally) is a design smell: split it into its hardware
primitive plus a program before extending it. Standing, user-approved
exception: the Wi-Fi Access Point block (hostapd in Java) stays as it is.

Scenarios are real player procedures. Each one must show the steps a player
takes to do the thing: real blocks placed and used by `ScenarioPlayer` through
the game's own interaction code, real computers running real programs, and
input given the way a player gives it (typing, clicking, keys, items). Do not
emulate parts of the setup with invisible or virtual stand-ins (no virtual
stations, hidden helper computers, unseen helper players, injected packets or
direct calls into mod internals to make something happen), and do not cheat a
source or a result into place. Reading internal state to *check* a result is
fine; *causing* the result that way is not. If a step genuinely cannot be done
by a player-equivalent action, write the reason in the scenario's notes and in
`docs/SCENARIO_EXCEPTIONS.md`; the only standing exception is scripted
airship/vehicle motion.

World-generation tests must exercise normal chunk generation in a fresh world,
not only `/place structure`. Client-visible block changes require a real
Minecraft rendering check with a screenshot and paired server/client assertions.
Use an isolated scripted server and hidden client, as described in ModTesting;
never automate the user's desktop, game profile, or saves for these tests.

Use the connected Blockbench MCP for new or substantially revised block models.
Keep editable `.bbmodel` sources, export game assets, inspect Blockbench previews,
and then inspect the Minecraft render. Neighbor-dependent models must have
tested placement, connection, removal, and reload behavior. Do not substitute
vanilla placeholder textures for a finished visual design.
