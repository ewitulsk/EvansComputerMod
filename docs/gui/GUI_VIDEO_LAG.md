# Why the GUI and video stutter, and what to do about it

Brainstorm, no code changed. Read from `origin/staging` at a1bf14f0 (2026-10-17). Every claim about the code cites a file; numbers marked *estimate* are worked out from sizes in the code, not measured.

## TL;DR

Nothing in the display pipeline *needs* the Minecraft server thread, but almost all of it runs there. Frames are diffed, compressed and sent from `server.execute(...)`, input and acks are handled with `enqueueWork` on the server thread, and the client applies frames on its render thread the moment they arrive, with no buffer. So any server tick hitch becomes a frozen screen, and any network jitter becomes a visible stutter. Video is the worst case because it is also shipped as raw changed pixels, which on a real server is tens of megabits per viewer.

**Recommendation, in order:**

1. **Take the display pipeline off the server thread** (sync, diff, deflate, acks, input). Cheapest change, biggest win in single player and on servers alike.
2. **Stream video as video, not as screen updates.** A "video node" in the scene protocol: frames carry timestamps, the client keeps a small jitter buffer and plays them on its own clock. Encode as JPEG (decoded in pure Java on the client, so the main mod stays native-free).
3. **Pipeline the tile path** (replace one-frame-in-flight with a window) so console graphics and Screen walls stop being capped at one frame per round trip.
4. **Predict simple GUI feedback on the client** (hover, press, scroll, caret, window drag), since the client already holds the scene tree.

## 1 · How a frame gets to the player today

There are two paths.

**Scene path** (GUI shown, client rendering on, the usual desktop case). The compositor ticks on its own scheduler (`UiManager.requestTick`, `TICKS` executor), composites on the server, and `SceneExporter` turns the change into a step: tree ops plus changed 16×16 tiles of every pixel surface and canvas (`SceneExporter.java:180-197`, which clones the whole canvas each frame as its shadow). The step is queued into `UiSceneHub.offer`, and `requestUiSync` → `requestDisplaySync` → `TerminalBlockEntity.syncToClients()` posts **`level.getServer().execute(...)`** (`TerminalBlockEntity.java:441-452`). On the server thread, `UiSceneHub.sync` deflates and sends. Up to 32 steps may be unacknowledged (`UiSceneHub.WINDOW`). The client applies each message on its main thread into `UiSceneMirror` and re-rasterizes the damage in Java.

**Tile path** (console mode, fullscreen programs like `player`, Screen walls for players not rendering scenes). `DisplaySyncHub.sync` diffs the `TerminalDisplay` against a shadow, deflates, sends, and then **waits for that viewer's ack before sending anything else** (`ClientSyncState.isClientReady`, one unacked frame, with a 5 s timeout). Same `server.execute` entry point.

**Video in the GUI player** (`rust/wasm-programs/videoplayer`). The app calls `video_decode_frame` (FFmpeg in the sidecar, on the program's thread), copies the frame into its own wasm memory, then `put_pixels` copies it back into a Java canvas (up to 400×240 ARGB, `MAX_DECODE_PIXELS`), commits, and the frame rides the scene path above as tile deltas. The app paces itself on a wall clock and drops up to 8 late frames (`main.rs:519-565`). Audio goes to the speaker on a separate path, so audio and video reach the client independently.

**Input.** `MouseInputPacket` and `KeyInputPacket` use `context.enqueueWork`, so every click and key waits for the server thread too. Acks (`TerminalReadyPacket`, `UiScenePackets.Ack`) do the same.

## 2 · Why lag hits the GUI and video hardest

1. **The server thread is a gate on every frame, input and ack.** `MinecraftServer.execute` from another thread queues the task; the server runs queued tasks between ticks. A 50 ms tick spike holds every frame for every computer for 50 ms, and when the server falls behind it spends less time draining the queue. Worse, the diff and deflate run on that same thread, so heavy displays make the ticks they wait on longer. The rest of Minecraft hides tick lag with client-side interpolation; a computer screen has nothing to interpolate, so the hitch shows directly.
2. **Frames are shown the instant they arrive.** There is no timestamp on a frame and no buffer on the client. When the server stalls, steps queue in `UiSceneHub.incoming`, then go out as a burst; the client applies them all in one render frame and shows only the last. That is the "freeze, then jump" you see. Network jitter on a real server does the same thing, all the time.
3. **Video is sent as raw changed pixels.** A 400×240 ARGB canvas is about 384 KB per frame before deflate, and nearly every tile changes every frame. At 30 fps that is roughly 11 MB/s raw, perhaps 4 to 8 MB/s after `BEST_SPEED` deflate (*estimate*), so 30 to 60 Mbit/s per viewer. On localhost that is free. On a real server it saturates the player's single Minecraft TCP connection, and chunks, entities, keep-alives and the acks themselves queue behind it (head-of-line blocking). Each extra viewer multiplies it on the server's upload.
4. **The tile path is capped at one frame per round trip.** With one frame in flight, the best case is 1 / (RTT + server queue wait + client frame wait). At 100 ms ping that is under 10 fps for console graphics, the fullscreen `player`, and Screen walls. A lost or dropped ack freezes that viewer for the 5 s `READY_TIMEOUT_MS`.
5. **"Under load" means contention in single player.** The integrated server, the client, the wasm worker and program threads, FFmpeg and the compositor all share one JVM and one CPU. Per-frame allocations (canvas clones, `readBytes` arrays, deflate buffers, the client's parse arrays) add GC pauses that stop both the server and the client at once. The compositor and the `ECM-Display-Sync` executor are each a single thread shared by every computer, so one slow computer delays the others.
6. **Video does a lot of copying on the server.** Decode → wasm memory → Java canvas → server composite (still done even when every viewer renders client-side) → clone into the exporter's shadow → diff → deflate. That is CPU on the machine that is already struggling.

I haven't profiled it, so which of these dominates your single-player freezes is inferred. My bet is 1 and 5 in single player, and 1, 2 and 3 on a dedicated server. A quick check is in §5.

## 3 · Options

### A. Get the pipeline off the server thread (do first)

- Run `DisplaySyncHub.sync` and `UiSceneHub.sync` on a display thread pool, not `server.execute`. `Connection.send` / `PacketDistributor.sendToPlayer` are safe from any thread (Netty does the encoding). The only thing that needs the server thread is *who is watching* (player positions, open menus), and that can be computed once per tick and published as a snapshot the display threads read.
- Handle acks and GUI input on the Netty thread straight into the hub and compositor, which are already synchronized. Only things that touch the world (redstone, block updates) need the server thread.
- On the client, inflate and parse on the Netty thread and ack on receipt; leave only the apply-to-texture step on the render thread.
- Give the compositor and display sync a small pool, not one shared thread, so computers don't block each other.
- Skip the server-side composite when every viewer renders the scene client-side and nothing else needs the pixels (headless screenshots, tile viewers).

Cost: modest, mostly plumbing. Risk: thread safety of viewer lists and `TerminalDisplay`, which are already locked. Fixes the "server hitch freezes the screen" class entirely.

### B. A real video stream (the big one for video)

Make video a first-class thing in the scene protocol instead of a canvas the app keeps repainting:

- The app says "show video handle H in this rectangle", plus play, pause and seek. It no longer copies every frame through its own memory.
- The sidecar decodes on its own thread and **encodes each frame as JPEG** at the target size (roughly 10 to 20 KB at 400×240, so 2 to 5 Mbit/s at 30 fps, *estimate*, about 10× less than today). Each frame carries its presentation time.
- The client decodes with the JDK's built-in `ImageIO` JPEG reader (part of Java, nothing native shipped by the mod, so it fits the CurseForge/Modrinth rule), keeps a jitter buffer of about 200 ms, and shows frames on its own clock. Network jitter and server hitches shorter than the buffer become invisible.
- Send the matching audio through the same stream with the same timestamps, so the client plays both off one clock and lip sync survives lag.
- Rate control: the server watches each viewer's acks and drops frames or lowers JPEG quality or size when a viewer falls behind, rather than queueing. Audio is never dropped.
- The server still has the frames for headless computers, screenshots, the Java rasterizer and tile-path viewers, at a lower rate if needed.

Alternatives considered: shipping the original H.264 to the client needs a native decoder on the client, which breaks the distribution rule unless it is an optional client sidecar (could be a later upgrade, with JPEG as the fallback). A pure-Java MPEG-1 decoder (jsmpeg-style) would cut bandwidth another 3 to 5× but is a real porting job. JPEG is the pragmatic first step.

The same "stream with timestamps and a jitter buffer" idea applies to any app that animates continuously (the GBA emulator, games, the waterfall SDR view), so build it as a general "timed pixel surface", with video as its first user.

### C. Pipeline the tile path

Replace `ClientSyncState`'s single in-flight frame with a window, as the scene path already does (`UiSceneHub.WINDOW = 32`), ideally sized in bytes so it adapts to the connection. Frame rate is then limited by bandwidth, not ping. Drop the 5 s ack timeout to something like 500 ms, or rely on the window plus a keyframe, so a lost ack can't freeze a viewer.

### D. Client-side prediction for the GUI

The client already holds the whole scene tree (`UiSceneMirror`) and animates window transitions locally. Extend that to immediate feedback: hover and pressed states, scrolling a list, moving the text caret, dragging a window. The client applies these locally and the server's next step confirms or corrects them. This hides the round trip that makes clicking feel sticky on a remote server. It needs care in `ecm-ui` to mark which widgets are safe to predict. Do it after A, B and C.

### E. Options I would not pick

- **Run the computer on the client.** It removes the latency but breaks the authority model (other players, networking, cheating) and needs wasmtime on the client, which the distribution rule forbids.
- **A separate UDP or TCP connection for media.** It avoids head-of-line blocking, but breaks behind proxies (Velocity, BungeeCord), hosting panels and firewalls. Rate control in B gets most of the benefit inside the normal connection.

### B′. Keep the framebuffer model instead: timed present (preferred after discussion with Evan, 2026-10-17)

Same result as B without a bypass, so video stays an ordinary framebuffer or canvas program (the VGA-style design intent):

- **`present` takes a timestamp** (e.g. `gfx_present_at(pts)`, and the same for canvas commits). The host keeps a short presentation queue; the client buffers and shows each frame on time. Video apps present a few hundred ms ahead; interactive programs present "now" and get no added latency. A flush call drops queued frames on seek/pause.
- **Transparent lossy transport:** the host JPEG-encodes surfaces that change heavily (detected, or hinted with something like `gfx_hint(video)`) and keeps exact tile deltas for text and UI.
- **Audio writes carry timestamps** on the same clock so the client can keep lip sync.
- Cost vs. B: a couple of extra frame copies through wasm memory, and no later path to forwarding the original H.264 (which would need a native client decoder anyway).

## 4 · Suggested order

| Step | What | Fixes |
| --- | --- | --- |
| 1 | A: pipeline off the server thread, acks and input on Netty, client parse off the render thread | Tick hitches freezing the screen; single-player stutter under load |
| 2 | C: windowed tile path, short ack timeout | Console graphics, fullscreen `player` and Screen walls capped by ping; 5 s freezes |
| 3 | B: timed video stream (JPEG + audio + jitter buffer + rate control) | Video stutter and the bandwidth wall on real servers |
| 4 | D: client-side prediction | Sticky-feeling clicks on high-ping servers |

## 5 · Measure before building

A quick check that would confirm the causes:

- Log per frame: when the step was made, when `server.execute` ran it, when the client received and applied it. The gap between the first two is the server-thread cost (cause 1); bursts at the client are cause 2.
- Log bytes per second per viewer while playing a video (`/ecm gui stats` already counts scene bytes) to confirm the bandwidth estimate in cause 3.
- Test on a dedicated server with artificial latency and jitter (for example `clumsy` on Windows, 100 ms ± 30 ms) instead of only in single player.
