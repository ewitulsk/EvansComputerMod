# Display pipeline: smooth GUI and video under lag (spec)

Status: proposal, 2026-10-17. Written against `origin/staging` at a1bf14f0. Background and the reasoning behind each choice are in [GUI_VIDEO_LAG.md](GUI_VIDEO_LAG.md); this file is the plan to build.

## 1 · Goal

A computer's screen keeps moving smoothly when the server hitches, the client hitches, or the network adds latency and jitter. Video plays without freezes and at a bandwidth a real multiplayer server can afford. Clicks and keys feel as fast as the round trip allows, never slower.

### Constraints

- **The framebuffer stays the interface.** Programs keep drawing with `gfx_*` (console, fullscreen, legacy windows in the GUI) or into UI canvases and pixel surfaces. There is no separate video path that bypasses them. Everything here is either a new optional call on that interface or a change in how frames travel.
- **No native code in the main mod, and none on the client.** Lossy encoding and decoding use the JDK's built-in `javax.imageio` JPEG codec. FFmpeg stays in the server-side sidecar.
- **Existing programs keep working unchanged** and get the threading and flow-control gains for free. Smooth timed playback is opt-in per program.
- **Server owners keep control** of bandwidth and quality through config. Defaults can only be lowered, as with panel tiers.

### Non-goals

- Running computers on the client.
- A second network connection (UDP or TCP) for media.
- Native client decoders (H.264 and the like). This could come later as an optional client sidecar, without changing anything below.

## 2 · Overview

```text
 program ── gfx_present_at(pts) / Commit{pts} ──▶ presentation queue (per display or surface)
                                                     │  frames go out as soon as queued, tagged with pts
                                                     ▼
                                  display pump threads (not the server thread)
                                   diff · choose encoding (exact tiles | JPEG) · deflate
                                   one build per frame, shared by every viewer
                                   windowed flow control + per-viewer rate control
                                                     │  normal game connection
                                                     ▼
 client network thread: inflate · parse · JPEG decode · ack on receipt
                                                     │
                                                     ▼
 client playout buffer: untimed updates now; timed frames at pts + playout delay
                                                     │
                                                     ▼
 render thread: apply to texture (changed tiles only) · draw
 speaker audio: same clock, same playout delay
```

The work splits into five parts, built in order (§10).

## 3 · Part A: the pipeline leaves the server thread

Today `TerminalBlockEntity.syncToClients()`, `PhoneHost`'s sync, acks (`TerminalReadyPacket`, `UiScenePackets.Ack`), `MouseInputPacket`, `KeyInputPacket` and speaker audio (`SpeakerBlockEntity.serverTick`) all run on the server thread.

### A1 · Display pump

- A new `DisplayPump` per server: a small thread pool, size `min(4, cores / 2)` by default (config `display.pumpThreads`). It replaces the single shared `ECM-Display-Sync` executor and runs `DisplaySyncHub.sync` and `UiSceneHub.sync`.
- Work is keyed per computer, so one computer's passes never run in parallel with each other but different computers run concurrently.
- Sends use `PacketDistributor.sendToPlayer` from pump threads. That call is safe off the server thread, and Netty does the encoding.
- `requestDisplaySync` keeps its refresh-rate pacing but schedules onto the pump instead of `server.execute`.

### A2 · Viewer snapshot

The only part that truly needs the server thread is deciding who watches what: player positions, open menus, Sable plot tracking and `canInteract`.

- Once per server tick, each host computes its viewer sets (`collectTerminalViewers`, `collectSyncRecipients`, the phone's holder and mirror viewers) and publishes them as an immutable snapshot through a volatile field.
- Pump passes read the latest snapshot. A viewer who left between ticks gets at most one extra frame, which is harmless.
- `UiManager.sceneViewers` and `UiClients.canRender` read the same snapshot.

### A3 · Acks and input off the server thread

- Ack handlers drop `enqueueWork` and go straight to the hubs, which are already synchronized. Before acting, they check the sender against the viewer snapshot, which stands in for the `ComputerTargets.resolve` range check.
- Mouse, key and text input go straight to the compositor (`Compositor` is synchronized) or the kernel input queue, also checked against the snapshot. Only input that changes the world (none today) would hop to the server thread.
- Tier 3 wall taps keep their range check against the snapshot.

### A4 · Speaker audio

`SpeakerBlockEntity` encodes and sends from the pump on its own audio cadence of about 20 ms, not from `serverTick`. The listener set comes from the A2 snapshot. Voice/SoundField emission stays on the tick.

### A5 · Client side

- `TerminalDeltaPacket`, scene messages and speaker packets are inflated, parsed and (Part D) JPEG-decoded on the network thread or a client decode thread. Only "apply to texture and draw" runs on the render thread.
- Acks go out on receipt, from the network thread, not after the render thread applies the frame.
- Texture updates upload only the changed tiles (`NativeImage` sub-region upload), if they don't already (`DISPLAY_PIPELINE.md` "Worth fixing" #3).

### A6 · Server-side compositing only when needed

`Compositor.tick` skips `composite()` when every viewer of the display renders the scene client-side and nothing else needs the pixels. Something else needs them when there are tile-path viewers, a screenshot or capture is requested, or a headless check runs. The composite happens lazily when one of those asks.

### A7 · Fewer per-frame allocations

`SceneExporter` canvas shadows, `DisplayDevice` blit buffers and delta builders reuse pooled arrays instead of `clone()` and `new byte[]` per frame. This matters most in single player, where GC pauses stop the server and the client together.

## 4 · Part B: windowed flow control

### B1 · Tile path

`ClientSyncState` replaces "one unacked frame" with a window, as the scene path already has:

- At most `display.windowFrames` frames (default 8) **and** `display.windowBytes` bytes (default 512 KB) unacknowledged per viewer. When either limit is hit, changes coalesce into the next frame, as today.
- `READY_TIMEOUT_MS` drops from 5 s to 1 s. On timeout the viewer gets a keyframe, not a resend of the stale chain. A lost ack can never freeze a screen for more than about a second.
- `DisplaySyncHub`'s history (16 steps / 2 MB) already supports this, because each viewer's chain is replayed from its acked generation.

### B2 · Per-viewer rate estimate

Every ack carries the client's receive time, and the hub keeps a per-viewer estimate of RTT, jitter and delivered throughput. Part D uses these numbers, and `/ecm gui stats` reports them.

### B3 · Scene path

`UiSceneHub` keeps its 32-step window and gains the byte limit and the rate estimate. Add the same 1 s no-ack timeout. Today the scene viewer has none.

## 5 · Part C: timed presentation

This is the VGA-style answer to smooth video: a program says *when* a frame should be seen, the way a real presentation queue (Vulkan `VK_GOOGLE_display_timing`, Wayland `presentation-time`) works.

### C1 · Clock

- Presentation times are **microseconds on the computer's monotonic clock**: the same clock a program reads with WASI `clock_time_get(MONOTONIC)`. The host already maps it to `System.nanoTime()`, so the server converts with a fixed offset per computer.
- Every timed message to a client carries the server's send time in the same units, and the client keeps a smoothed `serverMinusClient` offset estimate (minimum-filtered over recent messages, like NTP). `UiSceneMirror.clockOffset` becomes this shared estimate, one per server connection.

### C2 · ABI: framebuffer (`abi/child-abi.toml`)

New imports in `[imports.gfx]`:

```toml
gfx_present_at = { params = ["i32", "i32", "i64"], results = ["i64"], doc = "(target, flags, pts_us) queue the back buffer to be shown at pts_us (monotonic clock). Double-buffered displays only. pts_us <= now shows it at once (= gfx_present). Blocks while the queue is full. flags 1 = wait for queue space instead of failing with -11 (EAGAIN). -> frames queued" }
gfx_queue_flush = { params = ["i32"], results = ["i32"], doc = "(target) drop every queued frame not yet shown (seek, pause, stop). -> frames dropped" }
gfx_set_hint    = { params = ["i32", "i32"], results = ["i32"], doc = "(target, hint) 0 auto, 1 exact (never lossy), 2 motion (video: lossy allowed)" }
```

- **Queue limits:** at most `display.queueFrames` frames (default 12) and `display.queueMs` ahead of now (default 500 ms). `gfx_info` gains `queued` and `queue_ms` in what is now padding.
- `gfx_present` is unchanged: it means "show now".
- **Single-buffered displays** can't queue (a blit is visible at once), so `gfx_present_at` returns -22 (EINVAL) for them.
- `video_decode_to_gfx` keeps its behaviour. A program that wants queued video either uses `video_decode_frame` + `gfx_blit_rect` + `gfx_present_at`, or opens the display double-buffered and calls the new `video_decode_to_back(handle, target) -> pts_ms`, which decodes into the back buffer without presenting, followed by `gfx_present_at`.

### C3 · ABI: UI protocol (`abi/ui-protocol.toml`, version 2)

- New message **`CommitAt`** (`a>c`, fields `surface: u32`, `pts_us: u64`): like `Commit`, but the pixel changes to canvases and pixel surfaces in it take effect at `pts_us`. Tree changes in a `CommitAt` are an error (`Error{kind=invalid}`). Layout stays immediate, so text and buttons never wait behind video.
- New message **`QueueFlush`** (`a>c`, field `surface: u32`): drop the surface's queued commits.
- New message **`SurfaceSetContent`** (`a>c`, fields `surface: u32`, `resource: u32` (0 = the surface itself), `hint: u8`): the same 0/1/2 hint as `gfx_set_hint`, for a pixel surface or one canvas.
- `Welcome` reports `max_queue_frames` and `max_queue_ms`. Version 1 apps keep working, because `Commit` is unchanged.

### C4 · Server behaviour

- Each `DisplayDevice` (including `LegacyWindow` devices) and each timed surface or canvas in the compositor holds a **presentation queue** of frames, each a pixel copy plus its pts.
- **Frames go to clients as soon as they are queued**, tagged with their pts, so the client's buffer fills ahead of time. The diff base is the previous frame in pts order.
- The server's own `TerminalDisplay` / compositor state applies each frame **at its pts** (a pump timer), so the server always holds "what is on screen now". Headless computers, screenshots, the Java rasterizer, tile viewers and late joiners' keyframes all see the right frame.
- Late joiners get a keyframe of the current frame plus the rest of the queue.
- `gfx_queue_flush` / `QueueFlush` drops the queue on the server and sends a **flush marker** (timed stream id + "drop everything after generation G") to clients.
- `Frame` events (`RequestFrame`) for a timed surface fire when queue space opens, not on the refresh tick, so a producer runs exactly as far ahead as the queue allows.

### C5 · Wire format

- **Tile path:** `TerminalDeltaPacket` gains a flags byte with *timed* (followed by `pts_us`, `server_send_us`) and *encoding* (exact tiles or JPEG, Part D). Untimed packets keep today's layout, with flags = 0.
- **Scene path:** timed pixel data does **not** ride the ordered step stream, because a frame due in 400 ms would hold up a menu opening now. New `UiSceneCodec` message `TIMED_PIXELS` {target (surface/canvas), scope, id, stream generation, `pts_us`, `server_send_us`, encoding, data}. It is ordered per canvas and acked like steps. A step that resizes or destroys the canvas implicitly flushes its timed queue on the client.

## 6 · Part D: lossy motion encoding

### D1 · When frames go lossy

Per display, surface or canvas, the effective mode is:

| Hint | Mode |
| --- | --- |
| 1 exact | always exact tiles (today) |
| 2 motion | JPEG while frames keep changing |
| 0 auto (default) | JPEG once at least 50% of tiles changed in 8 of the last 10 frames; back to exact after 1 s below that |

- Text planes are always exact. In gfx mode 2 (graphics with the console on top) only the graphics plane can go lossy.
- Tier 1 panels never go lossy: they are small, and the client's two-tone filter would amplify artifacts.
- `display.allowLossy = false` (server config) turns the whole part off.

### D2 · Encoding

- The frame (or the bounding box of changed tiles, if under half the frame) is converted to RGB and JPEG-encoded with `javax.imageio` on the pump, **once per frame and shared by every viewer**, like today's deltas.
- **Quality ladder:** 0.85 / 0.75 / 0.6 / 0.45, plus half-resolution at the bottom. The hub picks one rung per frame for the slowest group of viewers. Viewers whose B2 estimate can't keep up get every 2nd or 3rd frame instead (frame dropping happens on the server and is never queued).
- **Refine:** when a lossy surface stops changing for 250 ms, one exact keyframe follows, so a paused video or the last frame looks crisp.
- Budget: `display.maxStreamKbps` per viewer (default 6000), shared across everything one player watches.

### D3 · Client

Decode on the client decode thread (`ImageIO.read` into a reused `BufferedImage`), convert to the texture's format, and hand it to the render thread as a tile update. Palette displays (INDEXED8) are decoded as RGB, so the client keeps a palette-free texture for lossy frames.

## 7 · Part E: client playout and audio on one clock

### E1 · Playout buffer

- **Untimed updates** (all of today's traffic, UI steps, interactive programs presenting "now") are applied as soon as they arrive, exactly as today. No added latency.
- **Timed frames** are shown at `pts + offset + playoutDelay`, where `playoutDelay` is per computer: `clamp(2 × jitter + 40 ms, 60 ms, 400 ms)`, raised quickly on lateness and lowered slowly. Overrides: client option `display.playoutDelayMs`, server cap `display.maxPlayoutMs`.
- **Late frames:** if a frame arrives after its slot, show it at once unless a newer frame is already due, in which case drop it. Count drops for `/ecm gui stats`.
- **Render-thread hitches:** on each render frame, show the newest frame that is due. Never apply a backlog one by one.

### E2 · Speaker audio

- `SpeakerAudioPacket` gains `pts_us` for the block's first sample. The server knows when each sample plays, because the speaker already has a queue (`latency`), so `pts = now + buffered_ms`. Programs don't change: `videoplayer`'s `AudioPump` already writes audio aligned to its media clock.
- The client schedules each audio block at `pts + offset + playoutDelay`, using **the playout delay of the computer the speaker is attached to**, so sound and picture shift together. That only applies while that computer has an active timed stream (a timed frame within the last 2 s). Otherwise speakers keep today's low-latency behaviour, so beeps and terminal sounds don't get slower.
- An audio gap or late block is handled as today (`seq`), plus "skip to pts" when more than 100 ms late.

## 8 · Part F: client-side GUI feedback (last)

The client already holds the scene tree and animates window transitions locally (`UiSceneMirror`). Extend it so common feedback doesn't wait a round trip:

- **Interaction states:** nodes may carry optional `hover` and `pressed` style overrides (fill, stroke, text colour). The client applies them under the pointer at once, and the server's own state arrives later and replaces them. This is a protocol change (new optional node fields), so it needs Java, `ecm-ui`, and the Python and Lua `ui` modules.
- **Scroll containers:** a group node flag `scroll` with a content size. The client scrolls locally, reports the offset to the app (`Scrolled{surface, node, x, y}`), and the app's next commit confirms it.
- **Window drag and resize:** the client moves the window frame locally while dragging and sends the final position. The compositor's answer reconciles it.
- **Text caret and selection** inside text fields, drawn locally while typing. The app's commit replaces it.

## 9 · Programs, scripting parity, docs

### Programs

- **`videoplayer`:** set the canvas hint to motion, decode about 300 ms ahead, `CommitAt(pts)` each frame, and `QueueFlush` on seek and pause. It keeps its wall clock, which becomes the producer side only.
- **`player`:** open double-buffered, `video_decode_to_back` + `gfx_present_at`, and `gfx_queue_flush` on interrupt.
- **Unchanged:** the GBA emulator and other interactive programs. They benefit from Parts A and B without code changes.

### Python and Lua

- The `ui` module's `Canvas` (and pixel surfaces) gain `commit_at(pts_us)`, `queue_flush()` and a `content` hint, identically in Python (`rust/wasm-programs/python/src/ui.py`) and Lua (`rust/wasm-programs/lua`), with tests in both and `lua_offers_every_python_module_and_name` passing.
- Any gfx or video functions exposed to Python get their Lua counterpart in the same PR.

### Docs (in each PR, per AGENTS.md)

- `abi/child-abi.toml` and `abi/ui-protocol.toml` doc strings.
- Man pages and `help` entries: `gfx`, `video`, `player`, `videoplayer`, the `ui` module pages.
- `docs/gui/DISPLAY_PIPELINE.md` "Pipeline today" section and diagram, `docs/gui/UI_PROTOCOL.md` for version 2, and the config keys.

## 10 · Phases (one PR each)

| PR | Content | User-visible result |
| --- | --- | --- |
| 1 | Part A (pump, snapshot, acks, input, speaker cadence, client parse thread, lazy composite, pooling), Part B, `/ecm gui stats` additions | No more freezes from server tick hitches; GUI input no longer waits for ticks; console graphics and walls are no longer capped by ping |
| 2 | Part C, Part E1, clock sync; `player` and `videoplayer` use queues; Python and Lua `ui` | Video smooth through server hitches and network jitter |
| 3 | Part D | Video bandwidth about 10× lower; viable on real servers |
| 4 | Part E2 | Lip sync under latency |
| 5 | Part F | Clicks, hovers and scrolling feel instant at high ping |

## 11 · Config (`EcmConfig`, server unless noted)

| Key | Default | Meaning |
| --- | --- | --- |
| `display.pumpThreads` | min(4, cores/2) | Display pump size |
| `display.windowFrames` / `display.windowBytes` | 8 / 512 KB | Unacked frames per viewer |
| `display.queueFrames` / `display.queueMs` | 12 / 500 | Presentation queue limits |
| `display.allowLossy` | true | Part D on or off |
| `display.maxStreamKbps` | 6000 | Per-viewer budget across all displays |
| `display.maxPlayoutMs` | 400 | Cap on the client playout delay |
| `display.playoutDelayMs` (client) | auto | Fixed playout delay instead of the adaptive one |

## 12 · Measuring and acceptance

`/ecm gui stats` gains, per viewer and display: pump time per pass, server-thread time spent on displays (should be about 0), RTT, jitter, delivered kbps, current quality rung, playout delay, frames on time, late and dropped, and audio skew against video.

Acceptance, all on a **dedicated server** as well as single player:

1. With 400×240 video at 30 fps under `/tick rate 5` (heavy server lag), at least 95% of frames are shown on time and none of the display work runs on the server thread.
2. At 100 ms ± 30 ms latency, the same video plays with at least 95% of frames on time, at 6 Mbit/s or less per viewer, with audio within 45 ms of picture.
3. A GUI click's visible response arrives within RTT + one client frame + one compositor tick, never waiting on a server tick.
4. An interactive program presenting "now" has no added delay versus today.
5. Two players watching one Screen wall cost one encode per frame (`deltaBuilds` / JPEG builds don't scale with viewers).

## 13 · Scenarios (`/ecm scenario`, shared with GameTests)

All are real player procedures: blocks placed by `ScenarioPlayer`, real computers, real programs, input typed and clicked. Each gets `auto` / `manual` / `fast` where sensible and full `/ecm scenario commands` text.

- **`video_smooth_under_lag`:** a player places a Terminal and a Speaker, opens the GUI and plays a sample video in `videoplayer`, then runs vanilla `/tick rate 5`. Checks: frames on time, server-thread display time, audio skew. Control: the same with `display.allowLossy=false` and the queue unused (today's behaviour) shows the freezes.
- **`video_seek_flush`:** seek and pause while frames are queued. Checks: no stale frame appears after the seek, and audio restarts at the target.
- **`wall_two_viewers`:** a second player watches a Tier 2 wall playing `player` fullscreen. Checks: one encode per frame, both players on time.
- **`interactive_latency`:** the GBA emulator or `mousetest`, presenting "now". Checks: click-to-pixel time is unchanged or better versus the baseline.
- **`lossy_refine`:** pause a video. Checks: within 300 ms the paused frame is exact (no JPEG artifacts in the captured texture).
- **Network latency** can't be produced by a player action. Like the latency conditions in acceptance 2, it is simulated by a test-only delay on the client connection. This is an exception to the scenario rules and goes in the scenario notes and `docs/SCENARIO_EXCEPTIONS.md`.

These touch the video and display systems heavily, so per the project's testing rule the video scenarios run for PRs 1 to 4. The Tech Village and phone-store suites only run if a PR changes those systems.

## 14 · Open questions

1. **Default playout delay range** (60 to 400 ms adaptive). Fine for video, but should a computer with only a game running ever buffer? As specified it doesn't, because games present "now".
2. **JPEG vs. a pure-Java inter-frame codec.** JPEG frames are independent, so they're robust and simple, but about 3 to 5× bigger than MPEG-1-style inter-frame coding. Worth revisiting only if the 6 Mbit/s default proves too much for real servers.
3. **1.21.1 backport:** nothing here is version-specific except the packet registration and texture upload calls, which already go through Stonecutter. `/tick rate` exists on both versions (added in 1.20.3), so the lag scenario works on both.
4. **ePhone:** the phone uses the same hubs (`PhoneHost`), so it gets every part automatically. Its battery model may want lossy mode forced on while unplugged. Not proposed for now.
