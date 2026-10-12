package com.example.evanscomputermod.computer;

import com.example.evanscomputermod.network.TerminalDeltaPacket;

import org.jetbrains.annotations.Nullable;
import java.util.ArrayDeque;
import java.util.ArrayList;
import java.util.Collection;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.UUID;
import java.util.function.BiConsumer;
import java.util.zip.Deflater;

/**
 * Streams one display to every player watching it, building each frame's
 * delta once.
 *
 * <p>The hub keeps one shadow of the display (the <i>head</i>, at generation
 * {@code headGen}). A sync pass diffs the display against the head once,
 * compresses the result once and commits it as the next generation; every
 * viewer whose last frame is the previous head gets that same packet. A
 * viewer that fell behind (its ack was outstanding while frames went out to
 * others) is sent the packets it missed, in order, from a short history,
 * and only gets a keyframe when the history no longer reaches back to its
 * frame. Keyframes are built at most once per pass and shared too.
 *
 * <p>Before this, every viewer had its own shadow, diff and deflate, so two
 * players watching one Screen cost two identical delta builds per frame
 * (DISPLAY_PIPELINE.md, "Worth fixing in either design" #2).
 *
 * <p>Flow control is unchanged: one unacked send per viewer, coalescing
 * meanwhile. The head only advances in passes where some viewer can take a
 * frame, so changes coalesce when nobody is ready. Diffing and encoding hold
 * the display's monitor (the program and kernel threads write it under the
 * same monitor); packets are handed to {@code send} after it is released.
 * Server thread; the methods are synchronized so keyframe requests from
 * other threads are safe.
 */
public final class DisplaySyncHub {

    /** Most packets kept for viewers that fell behind. */
    static final int HISTORY_MAX = 16;
    /** Most compressed bytes kept for them (a keyframe is cheaper beyond this). */
    static final long HISTORY_MAX_BYTES = 2L << 20;

    private final byte displayId;
    private final FramebufferDiffTracker head = new FramebufferDiffTracker();
    private final Deflater deflater = new Deflater(Deflater.BEST_SPEED);
    private final Map<UUID, ClientSyncState> viewers = new HashMap<>();
    private final ArrayDeque<Step> history = new ArrayDeque<>();
    private long historyBytes;
    /** The display the head shadows; a different one restarts the stream with a keyframe. */
    @Nullable
    private TerminalDisplay bound;
    private boolean started;
    private long headGen;
    /** The display's sync version the head shadows (see TerminalDisplay#changedTilesSince). */
    private int headVersion;
    private int lastTilesCompared;
    private long deltaBuilds;
    private long keyframeBuilds;
    private long packetsSent;

    /** One generation step of the stream: the packet that takes a client from {@code from} to {@code to}. */
    private record Step(long from, long to, TerminalDeltaPacket packet, boolean keyframe) {}

    public DisplaySyncHub(int displayId) {
        this.displayId = (byte) displayId;
    }

    /**
     * Bring the given viewers up to date with {@code display}: a keyframe for
     * those due one, the missed packets for ready viewers behind the head,
     * nothing for viewers still waiting to ack. {@code send} is called once
     * per packet, in order, after all locks are released.
     */
    public void sync(UUID computerId, @Nullable TerminalDisplay display, Collection<UUID> viewerIds,
                     BiConsumer<UUID, TerminalDeltaPacket> send) {
        if (display == null || viewerIds.isEmpty()) return;
        List<UUID> to = new ArrayList<>();
        List<TerminalDeltaPacket> out = new ArrayList<>();
        synchronized (this) {
            if (display != bound) {
                bound = display;
                started = false;
                history.clear();
                historyBytes = 0;
                for (ClientSyncState st : viewers.values()) st.needsKeyframe = true;
            }
            boolean wantFrame = false;
            for (UUID id : viewerIds) {
                ClientSyncState st = viewers.computeIfAbsent(id, k -> new ClientSyncState());
                if (st.shouldSendKeyframe() || st.isClientReady()) wantFrame = true;
            }
            if (!wantFrame) return;

            synchronized (display) {
                Step step = advance(computerId, display);
                TerminalDeltaPacket keyframe = step != null && step.keyframe ? step.packet : null;
                for (UUID id : viewerIds) {
                    ClientSyncState st = viewers.get(id);
                    List<TerminalDeltaPacket> chain = null;
                    if (!st.shouldSendKeyframe()) {
                        if (!st.isClientReady() || st.frameGen >= headGen) continue;
                        chain = chainFrom(st.frameGen);
                    }
                    if (chain == null) {
                        if (keyframe == null) keyframe = buildKeyframe(computerId, display);
                        to.add(id);
                        out.add(keyframe);
                        st.markKeyframeSent(headGen);
                    } else {
                        for (TerminalDeltaPacket p : chain) {
                            to.add(id);
                            out.add(p);
                        }
                        st.markSent(headGen);
                    }
                }
            }
            packetsSent += out.size();
        }
        for (int i = 0; i < to.size(); i++) send.accept(to.get(i), out.get(i));
    }

    /**
     * Diff the display against the head and, if anything changed, commit the
     * next generation. A change of shape (text grid, mode, pixel format,
     * graphics size) is a keyframe step, since deltas carry no sizes.
     * Caller holds this and the display's monitor.
     */
    @Nullable
    private Step advance(UUID computerId, TerminalDisplay d) {
        long gen = headGen + 1;
        TerminalDeltaPacket packet;
        boolean keyframe;
        if (!started || !head.matchesGeometry(d)) {
            packet = TerminalDeltaPacket.createKeyframe(computerId, displayId, d, gen, deflater);
            headVersion = d.syncVersion();
            if (started) head.commitShadow(d);
            else head.initShadow(d);
            started = true;
            keyframe = true;
            keyframeBuilds++;
        } else {
            int mode = d.getDisplayMode();
            FramebufferDiffTracker.TextDelta text = head.computeTextDelta(d);
            java.util.BitSet changed = d.changedTilesSince(headVersion);
            headVersion = d.syncVersion();
            FramebufferDiffTracker.GfxDelta gfx = mode >= 1 ? head.computeGfxDelta(d, changed) : null;
            lastTilesCompared = mode >= 1 ? head.lastTilesCompared() : 0;
            boolean textChanged = !text.changedRowIndices().isEmpty() || text.scrollOffset() != 0
                    || head.cursorDiffers(d);
            boolean gfxChanged = gfx != null && (!gfx.changedTileIndices().isEmpty() || gfx.paletteChanged());
            if (!textChanged && !gfxChanged) return null;
            packet = TerminalDeltaPacket.createDelta(computerId, displayId, gen, mode,
                    text, gfx, d.getWidth(), deflater);
            head.commitShadow(d);
            keyframe = false;
            deltaBuilds++;
        }
        Step step = new Step(headGen, gen, packet, keyframe);
        headGen = gen;
        history.addLast(step);
        historyBytes += packet.compressedPayload().length;
        while (history.size() > HISTORY_MAX || (historyBytes > HISTORY_MAX_BYTES && history.size() > 1)) {
            historyBytes -= history.removeFirst().packet.compressedPayload().length;
        }
        return step;
    }

    /** A keyframe of the head (the display equals the head when this runs). */
    private TerminalDeltaPacket buildKeyframe(UUID computerId, TerminalDisplay d) {
        keyframeBuilds++;
        return TerminalDeltaPacket.createKeyframe(computerId, displayId, d, headGen, deflater);
    }

    /** The packets from generation {@code from} up to the head, or null if the history doesn't reach back. */
    @Nullable
    private List<TerminalDeltaPacket> chainFrom(long from) {
        List<TerminalDeltaPacket> chain = null;
        for (Step s : history) {
            if (chain == null) {
                if (s.from != from) continue;
                chain = new ArrayList<>();
            }
            chain.add(s.packet);
        }
        return chain;
    }

    /** Drop viewers that are no longer watching. */
    public synchronized void retainViewers(Collection<UUID> ids) {
        viewers.keySet().retainAll(ids);
    }

    /** A client acked {@code generation}. Returns false if it isn't a viewer here. */
    public synchronized boolean onClientReady(UUID viewer, long generation) {
        ClientSyncState st = viewers.get(viewer);
        if (st == null) return false;
        st.onClientReady(generation);
        return true;
    }

    /** The viewer (added if new) gets a keyframe on the next pass. */
    public synchronized void requestKeyframe(UUID viewer) {
        viewers.computeIfAbsent(viewer, k -> new ClientSyncState()).needsKeyframe = true;
    }

    /** Every viewer gets a keyframe on the next pass. */
    public synchronized void forceKeyframe() {
        for (ClientSyncState st : viewers.values()) st.needsKeyframe = true;
    }

    /** Forget all viewers (the display was replaced); generations keep counting up. */
    public synchronized void reset() {
        viewers.clear();
        history.clear();
        historyBytes = 0;
        started = false;
        bound = null;
    }

    public synchronized int viewerCount() {
        return viewers.size();
    }

    /** Delta packets built so far (one per generation, however many viewers). */
    public synchronized long deltaBuilds() {
        return deltaBuilds;
    }

    /** Keyframe packets built so far. */
    public synchronized long keyframeBuilds() {
        return keyframeBuilds;
    }

    /** Packets handed to viewers so far (a shared packet counts once per viewer). */
    public synchronized long packetsSent() {
        return packetsSent;
    }

    /** Graphics tiles the last delta build compared, or -1 when it compared the whole plane. */
    public synchronized int lastTilesCompared() {
        return lastTilesCompared;
    }

    public synchronized long headGeneration() {
        return headGen;
    }
}
