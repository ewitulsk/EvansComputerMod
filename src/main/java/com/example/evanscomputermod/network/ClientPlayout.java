package com.example.evanscomputermod.network;

import java.util.ArrayDeque;
import java.util.Iterator;
import java.util.Map;
import java.util.UUID;
import java.util.concurrent.ConcurrentHashMap;

/**
 * Client side: the playout buffer for display packets (DISPLAY_PIPELINE_SPEC.md
 * Part E1). Plain Java, no client classes (the GameTest harnesses and unit
 * tests drive it the way the client does).
 *
 * <ul>
 *   <li><b>Untimed</b> packets (all console text, UI steps, programs that
 *       present "now") are applied the moment they arrive, exactly as before:
 *       no added latency.</li>
 *   <li><b>Timed</b> frames are held until {@code pts + playoutDelay} on this
 *       client's clock ({@link ClockSync}). The delay is per computer,
 *       {@code clamp(2 x jitter + 40 ms, 60 ms, display.maxPlayoutMs)}, raised
 *       at once when a frame arrives late and lowered slowly. The client
 *       option {@code display.playoutDelayMs} fixes it instead.</li>
 *   <li>A frame that arrives after its slot is shown at once, unless a newer
 *       frame is due too, which replaces it (counted as dropped). On each
 *       rendered frame only the newest due frame is shown; a backlog is never
 *       played one by one.</li>
 *   <li>A keyframe empties the buffer (frames after it are sent again); a
 *       flush marker drops the frames after its generation.</li>
 * </ul>
 * The speaker audio of a computer with a timed stream is scheduled with the
 * same delay ({@link #audioDelayUs}), so picture and sound shift together.
 *
 * <p>Main client thread (the decode thread hands packets over in order).
 */
public final class ClientPlayout {
    private ClientPlayout() {}

    /** Frames of a timed stream within this long count as "the computer streams" for audio (µs). */
    static final long TIMED_ACTIVE_US = 2_000_000;
    static final long MIN_DELAY_US = 60_000, BASE_DELAY_US = 40_000;

    /** The server's cap ({@code display.maxPlayoutMs}, from its display caps packet), ms. */
    public static volatile int maxPlayoutMs = 400;
    /** The client option {@code display.playoutDelayMs}: a fixed delay, or -1 for the adaptive one. */
    public static volatile int fixedDelayMs = -1;

    /** One computer's playout state and counters. */
    public static final class Computer {
        double jitterUs;
        double delayUs = 80_000;
        volatile long lastTimedUs = Long.MIN_VALUE / 2;
        long received, onTime, late, dropped, shown;

        public synchronized double delayMs() {
            return delayNowUs() / 1000.0;
        }

        /** The delay in use now: the client option if it is set, else the adaptive one. */
        synchronized long delayNowUs() {
            if (fixedDelayMs >= 0) return Math.min(Math.max(MIN_DELAY_US, maxPlayoutMs * 1000L), fixedDelayMs * 1000L);
            return (long) delayUs;
        }

        public synchronized double jitterMs() {
            return jitterUs / 1000.0;
        }

        /** {received, on time, late, dropped, shown}. */
        public synchronized long[] counters() {
            return new long[]{received, onTime, late, dropped, shown};
        }

        synchronized void arrived(long excessUs, long lateUs) {
            received++;
            jitterUs += (excessUs - jitterUs) / 16.0;
            long cap = Math.max(MIN_DELAY_US, maxPlayoutMs * 1000L);
            if (fixedDelayMs >= 0) {
                delayUs = Math.min(cap, fixedDelayMs * 1000L);
            } else {
                double target = Math.max(MIN_DELAY_US, Math.min(cap, 2 * jitterUs + BASE_DELAY_US));
                if (lateUs > 0) delayUs = Math.min(cap, delayUs + lateUs + 10_000);
                else if (target > delayUs) delayUs = target;
                else delayUs += (target - delayUs) * 0.02;
            }
            if (lateUs > 0) late++;
            else onTime++;
        }
    }

    private record Pending(TerminalDeltaPacket.ParsedDelta delta, long dueUs) {}

    private static final Map<UUID, Computer> COMPUTERS = new ConcurrentHashMap<>();
    private static final Map<ClientDisplayCache.Key, ArrayDeque<Pending>> QUEUES = new ConcurrentHashMap<>();

    public static Computer computer(UUID id) {
        return COMPUTERS.computeIfAbsent(id, k -> new Computer());
    }

    /** Stats of a computer, or null if it never streamed here. */
    public static Computer peek(UUID id) {
        return id == null ? null : COMPUTERS.get(id);
    }

    /**
     * How long to hold a computer's speaker audio (µs, client clock): its
     * playout delay while it has a timed stream (a timed frame within the last
     * 2 s), else 0 so beeps and terminal sounds keep today's low latency.
     */
    public static long audioDelayUs(UUID computer, long clientNowUs) {
        Computer c = computer == null ? null : COMPUTERS.get(computer);
        if (c == null || clientNowUs - c.lastTimedUs > TIMED_ACTIVE_US) return -1;
        return c.delayNowUs();
    }

    /**
     * A parsed packet for a display, received at client time {@code recvUs}.
     * Untimed: applied now. Timed: held until its time.
     */
    public static void receive(UUID computerId, int displayId, TerminalDeltaPacket.ParsedDelta d, long recvUs) {
        ClientDisplayCache.Key key = new ClientDisplayCache.Key(computerId, displayId);
        if (d.packetType() == TerminalDeltaPacket.PACKET_TYPE_FLUSH) {
            ArrayDeque<Pending> q = QUEUES.get(key);
            if (q != null) q.removeIf(p -> p.delta.generation() > d.flushAfter());
            return;
        }
        if (d.packetType() == TerminalDeltaPacket.PACKET_TYPE_KEYFRAME) {
            ArrayDeque<Pending> q = QUEUES.get(key);
            if (q != null) q.clear();
            ClientDisplayCache.apply(computerId, displayId, d);
            return;
        }
        if (!d.timed()) {
            // Graphics never overtake frames still waiting (an exact refine after a video must land after
            // its last frame); text doesn't wait (a timed frame never carries text).
            ArrayDeque<Pending> q = QUEUES.get(key);
            if (q != null && !q.isEmpty() && d.hasGfx()) {
                q.addLast(new Pending(d, q.peekLast().dueUs));
                return;
            }
            ClientDisplayCache.apply(computerId, displayId, d);
            return;
        }
        ClockSync clock = ClockSync.INSTANCE;
        clock.note(d.sendUs(), recvUs);
        Computer c = computer(computerId);
        long due = clock.toClient(d.ptsUs()) + c.delayNowUs();
        c.lastTimedUs = recvUs;
        c.arrived(clock.excessDelayUs(d.sendUs(), recvUs), Math.max(0, recvUs - due));
        QUEUES.computeIfAbsent(key, k -> new ArrayDeque<>()).addLast(new Pending(d, due));
    }

    /**
     * One rendered frame at client time {@code nowUs}: for every display,
     * the newest timed frame that is due is shown (the ones before it are
     * applied too, since frames build on each other, but were never on
     * screen).
     */
    public static void tick(long nowUs) {
        for (Map.Entry<ClientDisplayCache.Key, ArrayDeque<Pending>> e : QUEUES.entrySet()) {
            ArrayDeque<Pending> q = e.getValue();
            if (q.isEmpty() || q.peekFirst().dueUs > nowUs) continue;
            int n = 0;
            ClientDisplayCache.Key k = e.getKey();
            while (!q.isEmpty() && q.peekFirst().dueUs <= nowUs) {
                ClientDisplayCache.apply(k.computerId(), k.displayId(), q.removeFirst().delta);
                n++;
            }
            Computer c = computer(k.computerId());
            synchronized (c) {
                c.shown++;
                c.dropped += n - 1;
            }
        }
    }

    /**
     * When a timed scene frame (a canvas or pixel surface, UI protocol v2) is
     * due on this client's clock, with the same clock and playout delay as
     * tile frames; notes its arrival for the delay and the counters.
     */
    public static long timedDue(UUID computerId, long ptsUs, long sendUs, long recvUs) {
        ClockSync clock = ClockSync.INSTANCE;
        clock.note(sendUs, recvUs);
        Computer c = computer(computerId);
        long due = clock.toClient(ptsUs) + c.delayNowUs();
        c.lastTimedUs = recvUs;
        c.arrived(clock.excessDelayUs(sendUs, recvUs), Math.max(0, recvUs - due));
        return due;
    }

    /** {@code buffers} timed scene frames were shown this render frame. */
    public static void noteShown(UUID computerId, int buffers) {
        Computer c = computer(computerId);
        synchronized (c) {
            c.shown += buffers;
        }
    }

    /** Timed frames waiting for a display (diagnostics, tests). */
    public static int queued(UUID computerId, int displayId) {
        ArrayDeque<Pending> q = QUEUES.get(new ClientDisplayCache.Key(computerId, displayId));
        return q == null ? 0 : q.size();
    }

    /** Leaving a world. */
    public static void clear() {
        QUEUES.clear();
        COMPUTERS.clear();
        ClockSync.INSTANCE.reset();
    }

    /** Drop a computer's queues (its display was invalidated). */
    public static void forget(UUID computerId) {
        for (Iterator<ClientDisplayCache.Key> it = QUEUES.keySet().iterator(); it.hasNext(); ) {
            if (it.next().computerId().equals(computerId)) it.remove();
        }
    }
}
