package com.example.evanscomputermod.testing;

//? if <=1.21.1 {
import com.example.evanscomputermod.block.TerminalBlockEntity;
import com.example.evanscomputermod.computer.TerminalDisplay;
import com.example.evanscomputermod.computer.ui.ClientUiScenes;
import com.example.evanscomputermod.computer.ui.UiClients;
import com.example.evanscomputermod.computer.ui.UiSceneCodec;
import com.example.evanscomputermod.computer.ui.UiSceneHub;
import com.example.evanscomputermod.network.ClientDisplayCache;
import com.example.evanscomputermod.network.ClientPlayout;
import com.example.evanscomputermod.network.ClockSync;
import com.example.evanscomputermod.network.TerminalDeltaPacket;
import com.example.evanscomputermod.network.UiScenePackets;
import com.example.evanscomputermod.speaker.ImaAdpcm;
import com.example.evanscomputermod.speaker.SpeakerAudioPacket;
import com.example.evanscomputermod.speaker.SpeakerBlockEntity;
import io.netty.buffer.ByteBuf;
import io.netty.buffer.Unpooled;
import net.minecraft.network.protocol.common.custom.CustomPacketPayload;

import java.util.ArrayList;
import java.util.List;
import java.util.Random;
import java.util.UUID;
import java.util.concurrent.ConcurrentLinkedQueue;
import java.util.concurrent.Executors;
import java.util.concurrent.ScheduledExecutorService;
import java.util.concurrent.TimeUnit;
import java.util.zip.CRC32;

/**
 * One player's client over a network with latency and jitter, played in the
 * server's JVM for the display scenarios (DISPLAY_PIPELINE_SPEC.md §13). It is
 * a test viewer of a Terminal display: every packet the display pump sends it
 * goes through the real codec, arrives after {@code latency ± jitter} in
 * order (one TCP connection), is parsed and decoded on its "network thread"
 * and acked on receipt (the ack takes the latency back), and is applied by the
 * real client code ({@link ClientPlayout}, {@link ClientUiScenes}) on its
 * "render thread", which runs at 60 Hz and records when the watched part of
 * the picture changes. Speaker blocks it hears are scheduled the way the
 * client's speaker does, so it can tell how far sound and picture drift apart.
 *
 * <p>This stands in for a real client on a real network: a player can't add
 * latency to their own connection. It is the exception written down in
 * docs/SCENARIO_EXCEPTIONS.md ("Simulated network latency"). The client state
 * is keyed by a computer id of its own, so two of them don't share a cache.
 */
public final class LaggyViewer implements AutoCloseable {
    public final UUID id = UUID.randomUUID();
    private final TerminalBlockEntity te;
    private final UUID computer, clientComputer;
    private final int display, clientDisplay;
    private final boolean scenes;
    private final ScheduledExecutorService net, render;
    private final ConcurrentLinkedQueue<Runnable> main = new ConcurrentLinkedQueue<>();
    private final Random random = new Random(1234);
    private volatile long latencyUs, jitterUs;
    private long lastDeliverNs;
    private volatile boolean closed;
    private SpeakerBlockEntity speaker;

    // What it measured.
    private volatile int[] region; // x, y, w, h in device pixels of the display
    private volatile long lastCrc;
    private final List<long[]> changes = new ArrayList<>(); // {client µs, crc}
    private long bytes, packets;
    private final long startUs = ClockSync.clientNowUs();
    private double maxSkewMs, skewSum;
    private long skewSamples;
    // Flushes: per timed target, the frames dropped (seq ranges) and frames that arrived anyway.
    private final java.util.Map<Long, Long> maxSeq = new java.util.HashMap<>();
    private final java.util.Map<Long, long[]> dropped = new java.util.HashMap<>();
    private long flushes, stale;

    private static int instances;

    private LaggyViewer(TerminalBlockEntity te, int display, boolean scenes, long latencyMs, long jitterMs) {
        this.te = te;
        this.computer = te.getComputerId();
        this.display = display;
        this.scenes = scenes;
        int n;
        synchronized (LaggyViewer.class) {
            n = ++instances;
        }
        this.clientComputer = UUID.nameUUIDFromBytes(("laggy:" + computer + ":" + n).getBytes());
        this.clientDisplay = display;
        this.latencyUs = latencyMs * 1000;
        this.jitterUs = jitterMs * 1000;
        this.net = Executors.newSingleThreadScheduledExecutor(r -> daemon(r, "ECM-Test-Net-" + n));
        this.render = Executors.newSingleThreadScheduledExecutor(r -> daemon(r, "ECM-Test-Render-" + n));
        render.scheduleAtFixedRate(this::frame, 16, 16, TimeUnit.MILLISECONDS);
    }

    private static Thread daemon(Runnable r, String name) {
        Thread t = new Thread(r, name);
        t.setDaemon(true);
        return t;
    }

    /**
     * Watch display {@code display} of the Terminal: as a client that draws GUI
     * scenes itself ({@code scenes}) or one that gets tiles.
     */
    public static LaggyViewer watch(TerminalBlockEntity te, int display, boolean scenes, long latencyMs, long jitterMs) {
        LaggyViewer v = new LaggyViewer(te, display, scenes, latencyMs, jitterMs);
        if (scenes) UiClients.hello(v.id, UiSceneCodec.VERSION, ClientUiScenes.heldImageKeys());
        te.addTestViewer(v.id, display, v::sent);
        return v;
    }

    /** Hear a Speaker too (its blocks travel the same network). */
    public LaggyViewer hear(SpeakerBlockEntity s) {
        speaker = s;
        s.addTestListener(id, p -> sent(p));
        return this;
    }

    public void setNetwork(long latencyMs, long jitterMs) {
        latencyUs = latencyMs * 1000;
        jitterUs = jitterMs * 1000;
    }

    /** The part of the picture whose changes count as frames (device pixels); null = the whole display. */
    public void watchRegion(int x, int y, int w, int h) {
        region = new int[]{x, y, w, h};
    }

    // ------------------------------------------------------------ the network

    /** A packet the server sent this player: delivered after the network's delay, in order. */
    private void sent(CustomPacketPayload p) {
        if (closed) return;
        long now = System.nanoTime();
        long delay = latencyUs + (jitterUs == 0 ? 0 : (long) ((random.nextDouble() * 2 - 1) * jitterUs));
        long at;
        synchronized (this) {
            at = Math.max(lastDeliverNs, now + Math.max(0, delay) * 1000);
            lastDeliverNs = at;
        }
        net.schedule(() -> arrive(p), at - now, TimeUnit.NANOSECONDS);
    }

    /** Something going back to the server: it takes the latency too. */
    private void back(Runnable r) {
        if (closed) return;
        net.schedule(r, latencyUs, TimeUnit.MICROSECONDS);
    }

    /** The client's network and decode thread. */
    private void arrive(CustomPacketPayload p) {
        if (closed) return;
        long recv = ClockSync.clientNowUs();
        try {
            if (p instanceof TerminalDeltaPacket raw) {
                TerminalDeltaPacket t = roundTrip(raw, TerminalDeltaPacket.STREAM_CODEC);
                count(t.compressedPayload().length + 53);
                TerminalDeltaPacket.ParsedDelta d = t.parse();
                if (d.isJpeg()) d.decodeJpeg();
                long gen = d.generation();
                back(() -> te.onDisplayAck(id, display, gen, recv));
                main.add(() -> ClientPlayout.receive(clientComputer, clientDisplay, d, recv));
            } else if (p instanceof UiScenePackets.Scene s0) {
                UiScenePackets.Scene s = roundTrip(s0, UiScenePackets.Scene.STREAM_CODEC);
                count(s.data().length + UiSceneHub.FULL_OVERHEAD);
                scene(s.data(), recv);
            } else if (p instanceof UiScenePackets.Step s0) {
                UiScenePackets.Step s = roundTrip(s0, UiScenePackets.Step.STREAM_CODEC);
                count(s.data().length + UiSceneHub.STEP_OVERHEAD);
                scene(s.data(), recv);
            } else if (p instanceof SpeakerAudioPacket a0) {
                audio(roundTrip(a0, SpeakerAudioPacket.STREAM_CODEC), recv);
            }
        } catch (RuntimeException e) {
            com.example.evanscomputermod.EvansComputerMod.LOGGER.warn("laggy viewer: bad packet", e);
        }
    }

    private synchronized void count(long n) {
        bytes += n;
        packets++;
    }

    private static <T> T roundTrip(T p, net.minecraft.network.codec.StreamCodec<ByteBuf, T> codec) {
        ByteBuf buf = Unpooled.buffer();
        try {
            codec.encode(buf, p);
            return codec.decode(buf);
        } finally {
            buf.release();
        }
    }

    /** Per stream on this client: the epoch and generation acked on receipt. */
    private int ackedEpoch = -1;
    private long ackedGen = -1;

    private void scene(byte[] data, long recv) {
        UiSceneCodec.Message m;
        try {
            m = UiSceneCodec.unpack(data);
        } catch (Exception e) {
            m = null;
        }
        long already = -1;
        if (m != null && m.patch() != null) {
            if (m.kind() == UiSceneCodec.KEYFRAME) {
                ackedEpoch = m.epoch();
                ackedGen = m.patch().to();
                already = ackedGen;
                sceneAck(ackedEpoch, ackedGen, null, recv);
            } else if (m.kind() == UiSceneCodec.STEP && ackedGen >= 0 && ackedGen == m.patch().from()) {
                ackedGen = m.patch().to();
                already = ackedGen;
                sceneAck(ackedEpoch, ackedGen, null, recv);
            }
        } else if (m != null && m.timed() != null) {
            if (m.kind() == UiSceneCodec.TIMED) m.timed().decode();
            sceneAck(m.epoch(), UiSceneHub.TIMED_ACK | m.timed().tseq(), null, recv);
            noteTimed(m);
        }
        UiSceneCodec.Message msg = m;
        long acked = already;
        main.add(() -> {
            ClientUiScenes.Ack a = ClientUiScenes.onMessage(clientComputer, clientDisplay, msg, data.length,
                    System.nanoTime() / 1_000_000L, recv);
            if (a == null) return;
            if (a.generation() <= 0 || a.missing() != null) ackedGen = -1;
            else if (acked >= 0 && a.generation() <= acked) return;
            sceneAck(a.epoch(), a.generation(), a.missing(), recv);
        });
    }

    private void sceneAck(int epoch, long gen, long[] missing, long recv) {
        back(() -> te.onSceneAck(id, display, epoch, gen, missing, recv));
    }

    /**
     * Flush bookkeeping: a flush drops a target's frames after {@code after}
     * up to the newest one seen; a frame of that range arriving later would
     * be a stale frame shown after the flush.
     */
    private synchronized void noteTimed(UiSceneCodec.Message m) {
        UiSceneCodec.Timed t = m.timed();
        long key = ((long) t.scope() << 32) | (t.id() & 0xFFFFFFFFL);
        if (m.kind() == UiSceneCodec.TFLUSH) {
            flushes++;
            long top = maxSeq.getOrDefault(key, t.seq());
            if (top > t.seq()) dropped.put(key, new long[]{t.seq() + 1, top});
            return;
        }
        long[] d = dropped.get(key);
        if (d != null && t.seq() >= d[0] && t.seq() <= d[1]) stale++;
        maxSeq.merge(key, t.seq(), Math::max);
    }

    /** Flush markers received. */
    public synchronized long flushes() {
        return flushes;
    }

    /** Frames that arrived after a flush had dropped them (should be 0). */
    public synchronized long staleAfterFlush() {
        return stale;
    }

    /** The checksum of the watched region now. */
    public long lastChangeCrc() {
        return lastCrc;
    }

    /** A speaker block: when the client's speaker would play it, against the picture's delay. */
    private void audio(SpeakerAudioPacket a, long recv) {
        short[] pcm = ImaAdpcm.decode(a.data(), a.samples());
        if (pcm == null || a.ptsUs() == 0) return;
        main.add(() -> {
            long now = ClockSync.clientNowUs();
            UUID who = computer.equals(a.computer()) ? clientComputer : a.computer();
            long audioDelay = ClientPlayout.audioDelayUs(who, now);
            ClientPlayout.Computer c = ClientPlayout.peek(clientComputer);
            if (audioDelay < 0 || c == null) return; // no timed picture now: sound plays at once, as for beeps
            double skew = Math.abs(audioDelay / 1000.0 - c.delayMs());
            synchronized (this) {
                maxSkewMs = Math.max(maxSkewMs, skew);
                skewSum += skew;
                skewSamples++;
            }
        });
    }

    // ------------------------------------------------------------ the render thread

    private void frame() {
        if (closed) return;
        Runnable r;
        while ((r = main.poll()) != null) {
            try {
                r.run();
            } catch (RuntimeException e) {
                com.example.evanscomputermod.EvansComputerMod.LOGGER.warn("laggy viewer: apply failed", e);
            }
        }
        long nowUs = ClockSync.clientNowUs();
        ClientPlayout.tick(nowUs);
        ClientUiScenes.tick(System.nanoTime() / 1_000_000L);
        long crc = crc();
        if (crc != lastCrc) {
            lastCrc = crc;
            synchronized (this) {
                changes.add(new long[]{nowUs, crc});
            }
        }
    }

    private long crc() {
        TerminalDisplay d = display();
        synchronized (d) {
            byte[] px = d.getPixelData();
            if (px == null) return 0;
            int w = d.getGfxWidth(), h = d.getGfxHeight(), bpp = d.getBytesPerPixel();
            int[] r = region;
            int x0 = r == null ? 0 : Math.max(0, r[0]), y0 = r == null ? 0 : Math.max(0, r[1]);
            int x1 = r == null ? w : Math.min(w, r[0] + r[2]), y1 = r == null ? h : Math.min(h, r[1] + r[3]);
            CRC32 c = new CRC32();
            for (int y = y0; y < y1; y++) {
                int at = (y * w + x0) * bpp, n = (x1 - x0) * bpp;
                if (n > 0 && at + n <= px.length) c.update(px, at, n);
            }
            return c.getValue();
        }
    }

    // ------------------------------------------------------------ what it saw

    /** What this client shows. */
    public TerminalDisplay display() {
        return ClientDisplayCache.entry(clientComputer, clientDisplay).display;
    }

    /** Is the client drawing the display from a GUI scene? */
    public boolean drawsScene() {
        return ClientUiScenes.active(clientComputer, clientDisplay);
    }

    /** {received, on time, late, dropped, shown} of timed frames, or zeros. */
    public long[] timed() {
        ClientPlayout.Computer c = ClientPlayout.peek(clientComputer);
        return c == null ? new long[5] : c.counters();
    }

    /** Timed frames that arrived before their time (shown on time), as a fraction of those received. */
    public double onTimeFraction() {
        long[] t = timed();
        return t[0] == 0 ? 0 : t[1] / (double) t[0];
    }

    public double playoutDelayMs() {
        ClientPlayout.Computer c = ClientPlayout.peek(clientComputer);
        return c == null ? 0 : c.delayMs();
    }

    /** Changes of the watched region since {@code fromUs} (client clock). */
    public synchronized List<long[]> changesSince(long fromUs) {
        List<long[]> out = new ArrayList<>();
        for (long[] c : changes) if (c[0] >= fromUs) out.add(c);
        return out;
    }

    /**
     * How regular the picture was since {@code fromUs}: the fraction of
     * intervals between changes within {@code toleranceMs} of the median
     * interval (a video shown on time changes at its frame rate).
     */
    public double regularity(long fromUs, double toleranceMs) {
        List<long[]> c = changesSince(fromUs);
        if (c.size() < 4) return 0;
        List<Long> iv = new ArrayList<>();
        for (int i = 1; i < c.size(); i++) iv.add(c.get(i)[0] - c.get(i - 1)[0]);
        List<Long> sorted = new ArrayList<>(iv);
        java.util.Collections.sort(sorted);
        long median = sorted.get(sorted.size() / 2);
        long ok = iv.stream().filter(d -> Math.abs(d - median) <= toleranceMs * 1000).count();
        return ok / (double) iv.size();
    }

    /** Bytes received so far (with packet overhead). */
    public synchronized long bytes() {
        return bytes;
    }

    /** kbit/s received since {@code fromUs} given the bytes then. */
    public synchronized double kbpsSince(long fromUs, long bytesThen) {
        double s = Math.max(0.001, (ClockSync.clientNowUs() - fromUs) / 1e6);
        return (bytes - bytesThen) * 8 / 1000.0 / s;
    }

    /** The largest and mean gap between the sound's playout delay and the picture's (ms). */
    public synchronized double[] audioSkewMs() {
        return new double[]{maxSkewMs, skewSamples == 0 ? 0 : skewSum / skewSamples, skewSamples};
    }

    public synchronized void resetAudioSkew() {
        maxSkewMs = skewSum = 0;
        skewSamples = 0;
    }

    public static long nowUs() {
        return ClockSync.clientNowUs();
    }

    @Override
    public void close() {
        closed = true;
        te.removeTestViewer(id);
        if (speaker != null) speaker.removeTestListener(id);
        UiClients.forget(id);
        net.shutdownNow();
        render.shutdownNow();
        ClientPlayout.forget(clientComputer);
    }
}
//?}
