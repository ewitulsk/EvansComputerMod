# GUI Engine Docs — EvansComputerMod

Design docs for the compositor, swappable shell, app platform and ePhone on `feature/gui-engine`.

- [DISPLAY_PIPELINE.md](DISPLAY_PIPELINE.md) — today's display path and its copies, the Java compositor with a swappable shell, UI trees, display panel tiers, and the ePhone and app decisions.
- [UI_PROTOCOL.md](UI_PROTOCOL.md) — draft 0.2 of the protocol between apps, the shell and the compositor: transport, surfaces, display-list trees, input, shell messages, permissions, lifecycle, services and the `ecm-ui` toolkit.
- [../../abi/ui-protocol.toml](../../abi/ui-protocol.toml) — the frozen wire contract (v1): every message, opcode, field and constraint; codecs and test vectors are generated from it.
- [APPS.md](APPS.md) — the app platform as built: `app.toml`, `.ecmpkg`, repositories and `pkg`, app identity, permissions and where they are enforced, saved state, ePhone background rules, and the interfaces the compositor and phone wire.
- [EPHONE.md](EPHONE.md) — the ePhone as built: hardware (P5a: item, held-item host and states, slots, battery and Charging Dock, rendering, visibility, open mode and hotkeys) and the phone GUI (P5b: phone shell, apps one at a time with iPhone lifecycle, hotkeys to the open app, refresh wakes and the audio exception, GBA app).
- [ROADMAP.md](ROADMAP.md) — phases P0–P8 with dependencies, work items, "Done when" criteria and the testing approach.
- [VISUAL_PROGRAMMER.md](VISUAL_PROGRAMMER.md) — the Visual Programmer app (P6): node editor, `.vpl` and code generation, the GUI designer, Run, Package, and what still needs the compositor and `pkg`.

## Decisions log

Items the pages mark as decided. Everything else in these docs is a proposal until implementation settles it.

**Display pipeline** ([§3–5](DISPLAY_PIPELINE.md#3--why-the-compositor-goes-in-java))

- The compositor runs in Java (Option A); the shell is the swappable part.
- Console mode stays as the boot default; the GUI is started like `startx`, and a key chord switches between console and GUI. There is a terminal app.
- Many resolutions: apps get a logical size and a scale, never a raw pixel count; `ecm-ui` lays out with compact, medium and wide breakpoints; raw-pixel apps get integer scaling and letterboxing.
- Alpha is allowed on every surface.
- UI trees are designed into the protocol from the start, even if the first release still sends pixels.

**Display panels** ([§6](DISPLAY_PIPELINE.md#6--display-panels-and-tiers))

- Three tiers (Phosphor, Color LCD, HD), with no holographic or transparent tier.
- A newly crafted Terminal comes with a Tier 1 panel. All recipes are vanilla.
- Existing Terminals and Screens become Tier 2 when a world is upgraded.

**ePhone** ([§7](DISPLAY_PIPELINE.md#7--ephone-decisions))

| Topic | Decision |
| --- | --- |
| When it runs | Held: awake. In the hotbar, not held: asleep, paused except for background refresh windows. Anywhere else: off, resumes when picked up. Confirmed. |
| Battery | Yes; servers can turn it off and set drain and charge rates. |
| Duplication | Not handled; two items with the same ID are the same computer. |
| Others seeing your screen | Yes, at three levels: server maximum (off, glow only, full screen), per-player share, per-player "render others". |
| Input: phone open | Every key: full keyboard, mouse as touch, all bindings. Escape closes the phone. |
| Input: in hand while playing | Only the 6 hotkey slots; unbound by default; movement keys never taken. |
| Which app gets the slots | The open app; plus an OS-level "switch app" key owned by the shell. |
| Slots | Panel slot (Tier 1–3), 2 module slots (one ships with Wi-Fi), dedicated SIM slot. |
| Cellular | In scope: LTE and mmWave modules, SIM cards, player-built cell towers. |
| Calls and voice | Later. |

**Apps** ([§8](DISPLAY_PIPELINE.md#8--app-decisions))

| Topic | Decision |
| --- | --- |
| Packages | A package format, a `pkg` package manager and an App Store app, browsing repository servers on the in-game network. |
| Permissions | "Allow this app to use …" prompts, reviewable and revocable in Settings. |
| Languages | Rust and Python both get `ecm-ui`. |
| Visual Programmer | Becomes a desktop app that builds GUI apps. |
| Background | iPhone-style: periodic wakes on a limited budget, plus declared, permission-gated exception modes. |
| Shared services | Notifications, clipboard, file picker and "open with", all in v1. |
| Default apps | Terminal, Files, Settings, Text Editor, Visual Programmer, App Store, GBA emulator. |

**Lifecycle** ([UI Protocol §11](UI_PROTOCOL.md#11--lifecycle))

- ePhones follow iPhone rules; Terminals behave like a PC (apps keep running, minimized windows stop getting `Frame` events).
