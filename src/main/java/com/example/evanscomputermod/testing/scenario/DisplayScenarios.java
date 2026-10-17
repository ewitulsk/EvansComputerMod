package com.example.evanscomputermod.testing.scenario;

//? if <=1.21.1 {
import com.example.evanscomputermod.EcmConfig;
import com.example.evanscomputermod.block.ModBlocks;
import com.example.evanscomputermod.block.ScreenBlock;
import com.example.evanscomputermod.block.TerminalBlockEntity;
import com.example.evanscomputermod.computer.DisplaySyncHub;
import com.example.evanscomputermod.computer.TerminalDisplay;
import com.example.evanscomputermod.computer.pipeline.DisplayPump;
import com.example.evanscomputermod.computer.ui.Compositor;
import com.example.evanscomputermod.speaker.SpeakerBlock;
import com.example.evanscomputermod.speaker.SpeakerBlockEntity;
import com.example.evanscomputermod.speaker.SpeakerContent;
import com.example.evanscomputermod.testing.LaggyViewer;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.world.level.block.Blocks;

import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.WeakHashMap;

/**
 * The display pipeline (docs/gui/DISPLAY_PIPELINE.md, DISPLAY_PIPELINE_SPEC.md
 * §13), spawnable with {@code /ecm scenario spawn <name>} and run as the
 * GameTests of {@code ecm_display}:
 *
 * <ul>
 *   <li>{@code video_smooth_under_lag}: the Video Player plays a video on the
 *       desktop while the server is slowed to 5 ticks a second with vanilla
 *       {@code /tick rate 5}, watched by a client 100 ± 30 ms away. Frames are
 *       queued for their time and travel as JPEG: at least 95 % arrive on time,
 *       the stream stays under 6 Mbit/s, sound and picture keep the same delay
 *       (within 45 ms) and no display work runs on the server thread. Control:
 *       {@code display.allowLossy=false} and the player started
 *       {@code --untimed} (the frames shown as they arrive, as before) cost at
 *       least twice the bandwidth and come in bursts.</li>
 *   <li>{@code video_seek_flush}: seeking and pausing while frames are queued:
 *       the client drops what it held (no frame from before the seek shows
 *       after it) and the sound goes on from the new place.</li>
 *   <li>{@code lossy_refine}: a paused video's picture is exact (no JPEG
 *       artefacts) on the client within 300 ms.</li>
 *   <li>{@code wall_two_viewers}: {@code player screen} on a Tier 2 Screen wall
 *       watched by two clients: one encode per frame whatever the number of
 *       viewers, both on time.</li>
 *   <li>{@code interactive_latency}: {@code mousetest} (a program that
 *       presents "now") follows the pointer on a client as fast as the network
 *       allows, also with the server at 5 ticks a second: no added delay.</li>
 * </ul>
 *
 * The clients are {@link LaggyViewer}s: real client code in the server's JVM
 * behind a simulated network (a player can't add latency to their own
 * connection; docs/SCENARIO_EXCEPTIONS.md "Simulated network latency"). The
 * video scenarios need the {@code evanscomputermod-ffmpeg} sidecar
 * ({@code scripts/Test.ps1 -Ffmpeg}).
 */
public final class DisplayScenarios {
    public static final Map<String, Scenario> ALL = new LinkedHashMap<>();

    static final String APP = VideoScenarios.APP;
    static final String VIDEO = VideoScenarios.VIDEO;
    static final String VIDEO_PATH = VideoScenarios.VIDEO_PATH;
    private static final BlockPos PC = new BlockPos(0, 1, 0);
    private static final BlockPos SPEAKER = PC.west();
    /** A 2x1 Tier 2 Screen wall in front of the Terminal's west side. */
    private static final List<BlockPos> WALL = List.of(PC.west(), PC.west().north());

    /** Per run: the clients and what was measured. */
    static final class State {
        LaggyViewer viewer, second;
        long fromUs, bytesThen, serverThreadThen, jpegThen, buildsThen, packetsThen;
        double onTime, kbps, regularity, timedRegularity, timedKbps;
        long[] timedThen = new long[5];
        List<Long> latencies = new ArrayList<>();
        long moveAtNs;
        int moveX, moveY;
        long crcBefore;
        long flushesThen;
        java.util.Set<Long> shownBefore = new java.util.HashSet<>();
        long samples;
    }

    private static final Map<ScenarioRun, State> STATES = java.util.Collections.synchronizedMap(new WeakHashMap<>());

    static State st(ScenarioRun r) {
        return STATES.computeIfAbsent(r, k -> new State());
    }

    private DisplayScenarios() {}

    // ------------------------------------------------------------ shared steps

    /** A Terminal with a Speaker beside it and the test video in its folder (as video_desktop). */
    private static Scenario.Builder videoDesk(String name, String description) {
        List<BlockPos> floor = PlayerKit.box(-3, 0, -2, 3, 0, 2);
        List<BlockPos> foot = new ArrayList<>(floor);
        foot.add(SPEAKER);
        var b = Scenario.builder(name, description)
                .asPlayer()
                .host("pc", PC, "-")
                .decor(PlayerKit.decor(foot, r -> PlayerKit.fill(r, floor, Blocks.SMOOTH_STONE.defaultBlockState()), r -> {
                    r.player().place(SpeakerContent.SPEAKER.get(), r.abs(SPEAKER), Direction.SOUTH,
                            s -> s.getValue(SpeakerBlock.FACING) == Direction.SOUTH);
                    // Exception (docs/SCENARIO_EXCEPTIONS.md): a binary video can't be typed; players copy it into the folder.
                    PlayerKit.dropFiles(r, "pc", "home/videos", VideoScenarios.RESOURCES, List.of(VIDEO));
                }, DisplayScenarios::cleanUp))
                .timeLimit(60_000)
                .realTime()
                .note("Setup (done for you): a Speaker west of the Terminal; " + VIDEO + " (6 s, 320x180, 15 fps, a 440 Hz tone)"
                        + " copied into <world>/computer-data/<id>/home/videos/ [exception: binary files enter a computer from"
                        + " outside the game; docs/SCENARIO_EXCEPTIONS.md].");
        b.await(VideoScenarios::videoInstalled, "the server has video support (evanscomputermod-ffmpeg)", 1_000);
        return b;
    }

    /** Leave the server as it was: normal tick rate, lossy on, the clients gone. */
    static void cleanUp(ScenarioRun r) {
        try {
            r.player().command("/tick rate 20");
        } catch (RuntimeException ignored) {
            // the scenario may be cleared without a player
        }
        EcmConfig.setDisplayAllowLossy(true);
        State s = STATES.remove(r);
        if (s != null) {
            if (s.viewer != null) s.viewer.close();
            if (s.second != null) s.second.close();
        }
        var u = GuiScenarios.ui(r);
        if (u != null) u.stop();
    }

    static SpeakerBlockEntity speaker(ScenarioRun r) {
        return r.level().getBlockEntity(r.abs(SPEAKER)) instanceof SpeakerBlockEntity s ? s : null;
    }

    /** The GUI on, the Video Player started on the video from the Terminal app (as a player does it). */
    private static Scenario.Builder openVideo(Scenario.Builder b) {
        b.send("pc", "gui")
                .expect("pc", "^gui: started", "the GUI starts with the desktop", "(?i)unknown command|no GUI device")
                .await(r -> GuiScenarios.comp(r) != null && GuiScenarios.comp(r).shellConnected() ? null : "no desktop yet",
                        "the desktop is connected", 15_000);
        GuiScenarios.launch(b, "Terminal", "terminal");
        GuiScenarios.terminalCommand(b, "gui launch " + APP + " " + VIDEO_PATH);
        b.await(r -> GuiScenarios.shows(r, APP, "Pause"), "the Video Player plays it", 30_000)
                .await(r -> VideoScenarios.timeText(GuiScenarios.comp(r), GuiScenarios.appWindow(r, APP)).startsWith("0:01") ? null
                        : "time " + VideoScenarios.timeText(GuiScenarios.comp(r), GuiScenarios.appWindow(r, APP)), "the time runs (0:01)", 10_000)
                .await(r -> GuiScenarios.locate(r, APP, "Pause") != null ? null : "the controls are covered: windows "
                                + GuiScenarios.comp(r).windows(), "the Video Player is in front", 5_000)
                .mutate(r -> GuiScenarios.key(r, 0x0F, 0), "press L: loop (the test video is 6 s long)");
        return b;
    }

    /** The Video Player's picture area in device pixels (below its top bar, above its controls). */
    static int[] picture(ScenarioRun r) {
        Compositor.WindowInfo w = GuiScenarios.appWindow(r, APP);
        if (w == null) return null;
        return new int[]{w.x(), w.y() + 24, w.w(), Math.max(1, w.h() - 74)};
    }

    /** A client that draws the GUI itself, {@code latency ± jitter} ms away, watching the Video Player's picture. */
    static void sceneClient(ScenarioRun r, long latency, long jitter) {
        State s = st(r);
        if (s.viewer != null) s.viewer.close();
        s.viewer = LaggyViewer.watch(r.terminal("pc"), TerminalBlockEntity.DISPLAY_OWN, true, latency, jitter);
        SpeakerBlockEntity sp = speaker(r);
        if (sp != null) s.viewer.hear(sp);
    }

    /** Start measuring now. */
    static void mark(ScenarioRun r) {
        State s = st(r);
        s.fromUs = LaggyViewer.nowUs();
        s.bytesThen = s.viewer.bytes();
        s.serverThreadThen = DisplayPump.serverThreadNanos();
        s.timedThen = s.viewer.timed();
        s.viewer.resetAudioSkew();
        int[] p = picture(r);
        if (p != null) s.viewer.watchRegion(p[0], p[1], p[2], p[3]);
    }

    /** What happened since {@link #mark}: on-time fraction of timed frames, kbit/s, regularity. */
    static void measure(ScenarioRun r) {
        State s = st(r);
        long[] t = s.viewer.timed();
        long got = t[0] - s.timedThen[0], onTime = t[1] - s.timedThen[1];
        s.onTime = got == 0 ? 0 : onTime / (double) got;
        s.kbps = s.viewer.kbpsSince(s.fromUs, s.bytesThen);
        s.regularity = s.viewer.regularity(s.fromUs, 20);
        r.say("§7  measured: " + got + " timed frames, " + Math.round(s.onTime * 100) + "% on time, "
                + Math.round(s.kbps) + " kbit/s, " + Math.round(s.regularity * 100) + "% of frame intervals regular, playout "
                + Math.round(s.viewer.playoutDelayMs()) + " ms, audio skew max " + Math.round(s.viewer.audioSkewMs()[0]) + " ms");
    }

    static String serverThreadIdle(ScenarioRun r) {
        long ns = DisplayPump.serverThreadNanos() - st(r).serverThreadThen;
        return ns == 0 ? null : "display work ran on the server thread for " + ns / 1000 + " µs";
    }

    // ------------------------------------------------------------ video_smooth_under_lag

    static Scenario smooth() {
        var b = videoDesk("video_smooth_under_lag",
                "a video in the Video Player stays smooth with the server at 5 ticks/s and a client 100 +- 30 ms away");
        openVideo(b);
        b.note("A client draws the GUI itself across a network of 100 ms +- 30 ms [exception: simulated network latency]")
                .mutate(r -> sceneClient(r, 100, 30), "connect the client (100 +- 30 ms)")
                .await(r -> st(r).viewer.drawsScene() ? null : "no scene yet", "the client draws the desktop", 10_000)
                .note("Play it again from the start (Pause, then Play) and slow the server: /tick rate 5")
                .mutate(r -> GuiScenarios.clickText(r, APP, "Pause"), "click Pause")
                .await(r -> GuiScenarios.shows(r, APP, "Play"), "paused", 5_000)
                .mutate(r -> r.player().command("/tick rate 5"), "/tick rate 5")
                .mutate(r -> GuiScenarios.clickText(r, APP, "Play"), "click Play")
                .await(r -> GuiScenarios.shows(r, APP, "Pause"), "playing", 5_000)
                .waitMs(400, "the client's buffer fills")
                .mutate(DisplayScenarios::mark, "start measuring")
                .waitMs(2_500, "2.5 s of video at 5 ticks/s")
                .mutate(DisplayScenarios::measure, "measure")
                .await(r -> st(r).onTime >= 0.95 ? null : Math.round(st(r).onTime * 100) + "% on time",
                        "at least 95% of the frames arrive in time for their slot", 1_000)
                .await(r -> st(r).kbps <= 6000 ? null : Math.round(st(r).kbps) + " kbit/s", "at most 6 Mbit/s for this viewer", 1_000)
                .await(r -> st(r).viewer.audioSkewMs()[2] > 0 && st(r).viewer.audioSkewMs()[0] <= 45 ? null
                        : "audio skew " + java.util.Arrays.toString(st(r).viewer.audioSkewMs()),
                        "the Speaker's sound keeps the picture's delay (within 45 ms)", 1_000)
                .await(DisplayScenarios::serverThreadIdle, "no display work ran on the server thread", 1_000)
                .await(r -> {
                    var h = GuiScenarios.ui(r).sceneHub();
                    return h.timedCounters()[1] > 0 ? null : "no JPEG frames " + java.util.Arrays.toString(h.timedCounters());
                }, "the moving picture travels as JPEG (the Video Player marks its canvas as motion)", 1_000)
                .mutate(r -> {
                    st(r).timedRegularity = st(r).regularity;
                    st(r).timedKbps = st(r).kbps;
                }, "keep the timed run's numbers")
                .note("Control: lossy off (an operator sets display.allowLossy = false) and the Video Player started --untimed:"
                        + " frames are shown when they arrive, as before")
                .mutate(r -> EcmConfig.setDisplayAllowLossy(false), "display.allowLossy = false")
                .mutate(r -> GuiScenarios.key(r, 0x3D, com.example.evanscomputermod.computer.ui.proto.UiProto.Mods.ALT), "Alt+F4: close it")
                .await(r -> GuiScenarios.appWindow(r, APP) == null ? null : "still open", "the Video Player closed", 5_000);
        // The Terminal app has the focus again once the player is closed.
        GuiScenarios.terminalCommand(b, "gui launch " + APP + " --untimed " + VIDEO_PATH);
        b.await(r -> GuiScenarios.shows(r, APP, "Pause"), "the old way: playing, untimed", 30_000)
                .waitMs(400, "settle")
                .mutate(DisplayScenarios::mark, "start measuring")
                .waitMs(2_500, "2.5 s of video at 5 ticks/s")
                .mutate(DisplayScenarios::measure, "measure")
                .await(r -> st(r).kbps >= 2 * st(r).timedKbps ? null
                        : Math.round(st(r).kbps) + " kbit/s exact vs " + Math.round(st(r).timedKbps) + " kbit/s lossy",
                        "control: exact frames cost at least twice the JPEG stream", 1_000)
                .await(r -> st(r).regularity < st(r).timedRegularity - 0.1 ? null
                        : "untimed " + Math.round(st(r).regularity * 100) + "% vs timed " + Math.round(st(r).timedRegularity * 100) + "%",
                        "control: shown on arrival, the frames come in bursts (much less regular than timed)", 1_000)
                .mutate(r -> r.player().command("/tick rate 20"), "/tick rate 20")
                .mutate(r -> EcmConfig.setDisplayAllowLossy(true), "display.allowLossy = true");
        return b.build();
    }

    // ------------------------------------------------------------ lossy_refine

    /** Null when the client's picture equals the server's pixel for pixel. */
    static String pictureExact(ScenarioRun r) {
        Compositor c = GuiScenarios.comp(r);
        int[] p = picture(r);
        if (c == null || p == null) return "no window";
        byte[] server = c.frameRgba();
        TerminalDisplay d = st(r).viewer.display();
        int dw = c.display().deviceW();
        int diff = 0;
        synchronized (d) {
            byte[] px = d.getPixelData();
            if (px == null || px.length != server.length) return "the client shows " + d.getGfxWidth() + "x" + d.getGfxHeight();
            for (int y = p[1]; y < p[1] + p[3]; y++) {
                for (int x = p[0]; x < p[0] + p[2]; x++) {
                    int i = (y * dw + x) * 4;
                    if (i + 2 >= px.length) continue;
                    if (px[i] != server[i] || px[i + 1] != server[i + 1] || px[i + 2] != server[i + 2]) diff++;
                }
            }
        }
        return diff == 0 ? null : diff + " pixels differ";
    }

    static Scenario refine() {
        var b = videoDesk("lossy_refine", "a paused video is exact on the client within 300 ms (no JPEG artefacts)");
        openVideo(b);
        b.mutate(r -> sceneClient(r, 0, 0), "connect a client that draws the GUI [exception: in-process client]")
                .await(r -> st(r).viewer.drawsScene() ? null : "no scene yet", "the client draws the desktop", 10_000)
                .waitMs(1_200, "1.2 s of video")
                .await(r -> GuiScenarios.ui(r).sceneHub().lossyOut() ? null : "nothing lossy out",
                        "the moving picture went to the client as JPEG", 2_000)
                .await(r -> pictureExact(r) != null ? null : "already exact", "control: while it moves, the client's picture is approximate", 1_000)
                .mutate(r -> GuiScenarios.clickText(r, APP, "Pause"), "click Pause")
                .mutate(r -> st(r).moveAtNs = System.nanoTime(), "note the time")
                .await(r -> GuiScenarios.shows(r, APP, "Play"), "paused", 2_000)
                .await(r -> {
                    String why = pictureExact(r);
                    return why == null ? null : why + "; hub timed " + java.util.Arrays.toString(GuiScenarios.ui(r).sceneHub().timedCounters())
                            + " lossy out " + GuiScenarios.ui(r).sceneHub().lossyOut() + ", client queue "
                            + com.example.evanscomputermod.computer.ui.ClientUiScenes.get(st(r).viewer.clientComputer(), 0).timedQueued();
                }, "the paused picture is exact on the client", 3_000)
                .await(r -> {
                    long ms = (System.nanoTime() - st(r).moveAtNs) / 1_000_000;
                    return ms <= 300 + 100 ? null : "took " + ms + " ms";
                }, "within 300 ms of the pause (plus a step's time)", 100);
        return b.build();
    }

    // ------------------------------------------------------------ video_seek_flush

    static Scenario seek() {
        var b = videoDesk("video_seek_flush",
                "seek and pause while frames are queued: the client drops them, the sound goes on from the new place");
        openVideo(b);
        b.mutate(r -> sceneClient(r, 60, 10), "connect a client 60 +- 10 ms away [exception: simulated network latency]")
                .await(r -> st(r).viewer.drawsScene() ? null : "no scene yet", "the client draws the desktop", 10_000)
                .waitMs(1_500, "1.5 s of video: frames are queued ahead")
                .mutate(r -> st(r).flushesThen = st(r).viewer.flushes(), "note the flushes")
                .mutate(r -> st(r).samples = VideoScenarios.speakerSamples(r), "note the Speaker's samples")
                .note("Seek back 10 s (Left arrow: back to the start) while frames are queued")
                .mutate(r -> GuiScenarios.key(r, 0x50, 0), "press Left: back 10 s, to 0:00")
                .await(r -> st(r).viewer.flushes() > st(r).flushesThen ? null : "no flush yet",
                        "the client got a flush marker and dropped the frames it held", 3_000)
                .await(r -> VideoScenarios.timeText(GuiScenarios.comp(r), GuiScenarios.appWindow(r, APP)).startsWith("0:00")
                        || VideoScenarios.timeText(GuiScenarios.comp(r), GuiScenarios.appWindow(r, APP)).startsWith("0:01") ? null
                        : "time " + VideoScenarios.timeText(GuiScenarios.comp(r), GuiScenarios.appWindow(r, APP)), "the time is back at the start", 3_000)
                .await(r -> st(r).viewer.staleAfterFlush() == 0 ? null : st(r).viewer.staleAfterFlush() + " stale frames",
                        "no frame queued before the seek was shown after it", 1_000)
                .await(r -> VideoScenarios.speakerSamples(r) - st(r).samples > 11_000 ? null : "speaker +" + (VideoScenarios.speakerSamples(r) - st(r).samples),
                        "the sound goes on from the new place", 5_000)
                .note("Pause while frames are queued: the queue is dropped and the picture stops")
                .mutate(r -> st(r).flushesThen = st(r).viewer.flushes(), "note the flushes")
                .mutate(r -> GuiScenarios.clickText(r, APP, "Pause"), "click Pause")
                .await(r -> GuiScenarios.shows(r, APP, "Play"), "paused", 3_000)
                .await(r -> st(r).viewer.flushes() > st(r).flushesThen ? null : "no flush", "the pause flushed the client's queue", 3_000)
                .waitMs(600, "a moment (a refine may follow)")
                .mutate(r -> st(r).crcBefore = st(r).viewer.lastChangeCrc(), "look at the picture")
                .waitMs(600, "0.6 s")
                .await(r -> st(r).viewer.lastChangeCrc() == st(r).crcBefore ? null : "the picture moved", "control: paused, it stands still", 1_000);
        return b.build();
    }

    // ------------------------------------------------------------ wall_two_viewers

    static Scenario wall() {
        List<BlockPos> floor = PlayerKit.box(-3, 0, -3, 3, 0, 2);
        List<BlockPos> foot = new ArrayList<>(floor);
        foot.addAll(WALL);
        var b = Scenario.builder("wall_two_viewers",
                        "player on a Tier 2 Screen wall watched by two clients: one encode per frame, both on time")
                .asPlayer()
                .host("pc", PC, "-")
                .decor(PlayerKit.decor(foot, r -> PlayerKit.fill(r, floor, Blocks.SMOOTH_STONE.defaultBlockState()), r -> {
                    for (BlockPos p : WALL) {
                        r.player().place(ModBlocks.SCREEN_BLOCK.get(), r.abs(p), Direction.WEST,
                                s -> s.getValue(ScreenBlock.FACING) == Direction.WEST);
                    }
                    PlayerKit.dropFiles(r, "pc", "home/videos", VideoScenarios.RESOURCES, List.of(VIDEO));
                }, DisplayScenarios::cleanUp))
                .timeLimit(60_000)
                .realTime()
                .note("Setup (done for you): two Tier 2 Screens against the Terminal's west side; " + VIDEO + " copied into"
                        + " the computer's folder [exception: binary files; docs/SCENARIO_EXCEPTIONS.md].");
        b.await(VideoScenarios::videoInstalled, "the server has video support (evanscomputermod-ffmpeg)", 1_000)
                .await(r -> r.terminal("pc").getScreenClusterInfo() != null ? null : "no wall", "the wall is attached", 10_000)
                .note("Two players watch the wall from 50 ms away [exception: in-process clients]")
                .mutate(r -> {
                    State s = st(r);
                    s.viewer = LaggyViewer.watch(r.terminal("pc"), TerminalBlockEntity.DISPLAY_ATTACHED, false, 50, 10);
                    s.second = LaggyViewer.watch(r.terminal("pc"), TerminalBlockEntity.DISPLAY_ATTACHED, false, 50, 10);
                }, "two clients in range of the wall")
                .send("pc", "player screen " + VIDEO_PATH)
                .await(r -> r.terminal("pc").getScreenDisplay() != null && r.terminal("pc").getScreenDisplay().getDisplayMode() >= 1 ? null
                        : "the wall shows nothing", "the video is on the wall", 15_000)
                .waitMs(500, "the queue fills")
                .mutate(r -> {
                    State s = st(r);
                    DisplaySyncHub h = r.terminal("pc").getSyncHub(TerminalBlockEntity.DISPLAY_ATTACHED);
                    s.buildsThen = h.timedBuilds() + h.deltaBuilds() + h.keyframeBuilds();
                    s.packetsThen = h.packetsSent();
                    s.timedThen = s.viewer.timed();
                }, "start counting")
                .waitMs(2_000, "2 s of video")
                .await(r -> {
                    State s = st(r);
                    DisplaySyncHub h = r.terminal("pc").getSyncHub(TerminalBlockEntity.DISPLAY_ATTACHED);
                    long builds = h.timedBuilds() + h.deltaBuilds() + h.keyframeBuilds() - s.buildsThen;
                    long packets = h.packetsSent() - s.packetsThen;
                    r.say("§7  " + builds + " builds, " + packets + " packets sent");
                    if (builds < 10) return "only " + builds + " frames built";
                    return packets >= 2 * builds - 4 && packets <= 2 * builds + 4 ? null
                            : builds + " builds for " + packets + " packets (expected two packets per build)";
                }, "one encode per frame, the same packet to both viewers", 1_000)
                .await(r -> st(r).viewer.onTimeFraction() >= 0.95 && st(r).second.onTimeFraction() >= 0.95 ? null
                        : "on time " + Math.round(st(r).viewer.onTimeFraction() * 100) + "% / " + Math.round(st(r).second.onTimeFraction() * 100) + "%",
                        "both viewers get at least 95% of the frames on time", 1_000)
                .await(r -> java.util.Arrays.equals(st(r).viewer.timed(), st(r).viewer.timed()) ? null : "?", "counted", 100);
        return b.build();
    }

    // ------------------------------------------------------------ interactive_latency

    /** Null once the client shows mousetest's circle at the pointer (a non-black pixel there). */
    static String circleAt(ScenarioRun r) {
        State s = st(r);
        long lit = s.viewer.litAtNs();
        if (lit == 0) return "not there yet";
        // Timed by the client's render thread when the pixel lit up (a step only runs once a tick).
        s.latencies.add((lit - s.moveAtNs) / 1_000_000);
        return null;
    }

    /** Move the pointer to a fresh spot (as the player's mouse does) and note when. */
    static void moveTo(ScenarioRun r, int i) {
        State s = st(r);
        s.moveX = 40 + (i * 53) % 220;
        s.moveY = 40 + (i * 37) % 110;
        s.viewer.armPixel(s.moveX, s.moveY);
        s.moveAtNs = System.nanoTime();
        r.player().move(r.computerId("pc"), TerminalBlockEntity.DISPLAY_OWN, s.moveX, s.moveY);
    }

    static long median(List<Long> v) {
        List<Long> c = new ArrayList<>(v);
        java.util.Collections.sort(c);
        return c.isEmpty() ? -1 : c.get(c.size() / 2);
    }

    private static Scenario.Builder pointerRounds(Scenario.Builder b, int from, String label) {
        b.mutate(r -> st(r).latencies.clear(), "start " + label);
        for (int i = from; i < from + 5; i++) {
            int k = i;
            b.mutate(r -> moveTo(r, k), "move the pointer");
            b.await(DisplayScenarios::circleAt, "the client shows the circle there", 3_000);
            b.waitMs(60, "a moment");
        }
        return b;
    }

    static Scenario interactive() {
        var b = Scenario.builder("interactive_latency",
                        "mousetest presents now: the pointer's circle reaches a client as fast as the network allows, also at 5 ticks/s")
                .asPlayer()
                .host("pc", PC, "-")
                .decor(PlayerKit.decor(PlayerKit.box(-2, 0, -2, 2, 0, 2),
                        r -> PlayerKit.fill(r, PlayerKit.box(-2, 0, -2, 2, 0, 2), Blocks.SMOOTH_STONE.defaultBlockState()),
                        r -> {}, DisplayScenarios::cleanUp))
                .timeLimit(60_000)
                .realTime();
        b.mutate(r -> {
                    State s = st(r);
                    s.viewer = LaggyViewer.watch(r.terminal("pc"), TerminalBlockEntity.DISPLAY_OWN, false, 0, 0);
                }, "a client watching the Terminal's screen [exception: in-process client]")
                .send("pc", "mousetest")
                .expect("pc", "move the mouse", "mousetest runs (it presents every frame now)", "(?i)not available|failed");
        pointerRounds(b, 0, "baseline: no network delay, 20 ticks/s");
        b.mutate(r -> {
            r.note("baseline", Long.toString(median(st(r).latencies)));
            r.say("§7  baseline click-to-pixel median " + median(st(r).latencies) + " ms " + st(r).latencies);
        }, "note the baseline");
        b.mutate(r -> r.player().command("/tick rate 5"), "/tick rate 5");
        pointerRounds(b, 5, "the server at 5 ticks/s");
        b.await(r -> {
            long m = median(st(r).latencies), base = Long.parseLong(r.noted("baseline"));
            r.say("§7  at 5 ticks/s: median " + m + " ms " + st(r).latencies);
            return m <= base + 40 ? null : "median " + m + " ms vs baseline " + base + " ms";
        }, "no slower than the baseline (within a frame): input and frames don't wait for ticks", 1_000);
        b.mutate(r -> r.player().command("/tick rate 20"), "/tick rate 20");
        b.mutate(r -> st(r).viewer.setNetwork(100, 30), "the client moves 100 +- 30 ms away [exception: simulated latency]");
        pointerRounds(b, 10, "100 +- 30 ms away");
        b.await(r -> {
            long m = median(st(r).latencies), base = Long.parseLong(r.noted("baseline"));
            r.say("§7  at 100 +- 30 ms: median " + m + " ms " + st(r).latencies);
            return m <= base + 130 + 40 ? null : "median " + m + " ms";
        }, "the network's delay and nothing more: no playout buffer for a program presenting now", 1_000)
                .await(r -> st(r).viewer.timed()[0] == 0 ? null : st(r).viewer.timed()[0] + " timed frames",
                        "control: nothing was timed (mousetest presents now)", 1_000)
                .send("pc", "\u0014");
        return b.build();
    }

    static {
        for (Scenario s : List.of(smooth(), seek(), refine(), wall(), interactive())) ALL.put(s.name, s);
    }
}
//?}
