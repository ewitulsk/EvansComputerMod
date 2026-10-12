package com.example.evanscomputermod.testing;

import com.example.evanscomputermod.computer.TerminalDisplay;
import com.example.evanscomputermod.computer.ui.ClientUiScenes;
import com.example.evanscomputermod.computer.ui.Compositor;
import com.example.evanscomputermod.computer.ui.UiClients;
import com.example.evanscomputermod.computer.ui.UiManager;
import com.example.evanscomputermod.computer.ui.UiSceneCodec;
import com.example.evanscomputermod.computer.ui.UiSceneHub;
import com.example.evanscomputermod.network.ClientDisplayCache;
import com.example.evanscomputermod.network.UiScenePackets;
import io.netty.buffer.ByteBuf;
import io.netty.buffer.Unpooled;
import net.minecraft.network.protocol.common.custom.CustomPacketPayload;

import java.util.ArrayList;
import java.util.List;
import java.util.UUID;

/**
 * Plays one player whose client draws a computer's GUI itself (UI_PROTOCOL.md
 * §14 phase 2a), headless, with the real code on both ends: the computer's
 * own {@link UiSceneHub} streams to it, every message goes through the real
 * packet codec ({@link UiScenePackets}, short stream ids included), and the
 * client side is the real {@link ClientUiScenes}, which draws into the
 * {@link ClientDisplayCache} entry the Terminal screen and Screen walls read.
 *
 * <p>It watches as display {@link #DISPLAY} (a key the server's own sync
 * passes, which only handle displays 0 and 1, leave alone), after checking
 * that the server would stream display 0 to it as a scene.
 */
public final class SceneViewerMirror {
    /** The hub key this viewer watches under. */
    public static final int DISPLAY = 9;

    private final UiManager ui;
    private final UUID computer;
    public final UUID player = UUID.randomUUID();
    private long messages;

    public SceneViewerMirror(UiManager ui, UUID computer) {
        this.ui = ui;
        this.computer = computer;
        UiClients.hello(player, UiSceneCodec.VERSION, ClientUiScenes.heldImageKeys());
    }

    public static long nowMs() {
        return System.nanoTime() / 1_000_000L;
    }

    /** Would the server stream the Terminal display to this player as a scene right now? */
    public boolean eligible() {
        return !ui.sceneViewers(0, List.of(player)).isEmpty();
    }

    /**
     * One server pass and the client's handling of what it sent (acks
     * included), then the client draws. Watches while eligible; otherwise
     * the pass ends the stream. Returns the messages delivered.
     */
    public int pump() {
        int n = 0;
        for (int round = 0; round < 8; round++) {
            List<CustomPacketPayload> sent = new ArrayList<>();
            ui.sceneSync(DISPLAY, eligible() ? List.of(player) : List.of(),
                    (p, d, data) -> sent.add(roundTrip(UiScenePackets.packet(p, computer, d, data))));
            if (sent.isEmpty()) break;
            for (CustomPacketPayload pkt : sent) {
                UUID id;
                int display;
                byte[] data;
                if (pkt instanceof UiScenePackets.Scene s) {
                    if (s.stream() != 0) ClientUiScenes.bindStream(s.stream(), s.computerId(), s.displayId());
                    id = s.computerId();
                    display = s.displayId();
                    data = s.data();
                } else {
                    UiScenePackets.Step s = (UiScenePackets.Step) pkt;
                    ClientUiScenes.Key k = ClientUiScenes.stream(s.stream());
                    if (k == null) throw new IllegalStateException("a step on stream " + s.stream() + " before it was bound");
                    id = k.computerId();
                    display = k.displayId();
                    data = s.data();
                }
                ClientUiScenes.Ack ack = ClientUiScenes.onMessage(id, display, data, nowMs());
                if (ack != null) ui.sceneHub().onAck(player, display, ack.epoch(), ack.generation(), ack.missing());
                n++;
            }
        }
        messages += n;
        ClientUiScenes.tick(nowMs());
        return n;
    }

    /** Serialise and parse a packet with its real codec. */
    private static CustomPacketPayload roundTrip(CustomPacketPayload p) {
        ByteBuf buf = Unpooled.buffer();
        try {
            if (p instanceof UiScenePackets.Scene s) {
                UiScenePackets.Scene.STREAM_CODEC.encode(buf, s);
                return UiScenePackets.Scene.STREAM_CODEC.decode(buf);
            }
            UiScenePackets.Step.STREAM_CODEC.encode(buf, (UiScenePackets.Step) p);
            return UiScenePackets.Step.STREAM_CODEC.decode(buf);
        } finally {
            buf.release();
        }
    }

    /** Is the client drawing this display from a scene? */
    public boolean active() {
        return ClientUiScenes.active(computer, DISPLAY) && ClientDisplayCache.entry(computer, DISPLAY).sceneDriven;
    }

    /** What the client's display shows (the pixels a Terminal screen would upload). */
    public TerminalDisplay display() {
        return ClientDisplayCache.entry(computer, DISPLAY).display;
    }

    /** Bytes the server sent this viewer so far. */
    public long bytes() {
        UiSceneHub.Viewer v = ui.sceneHub().viewer(player, DISPLAY);
        return v == null ? 0 : v.bytes();
    }

    public UiSceneHub.Viewer counters() {
        return ui.sceneHub().viewer(player, DISPLAY);
    }

    /**
     * Null if the client's display equals the compositor's frame pixel for
     * pixel, else how many pixels differ. Holds the compositor still (no
     * tick runs) while it pumps and compares.
     */
    public String compare(Compositor comp) {
        synchronized (comp) {
            pump();
            byte[] server = comp.frameRgba();
            TerminalDisplay d = display();
            synchronized (d) {
                byte[] px = d.getPixelData();
                if (px == null || px.length != server.length) {
                    return "the client display is " + d.getGfxWidth() + "x" + d.getGfxHeight() + " (server frame " + server.length / 4 + " px)";
                }
                int diff = 0, first = -1;
                for (int i = 0; i < server.length; i += 4) {
                    if (server[i] != px[i] || server[i + 1] != px[i + 1] || server[i + 2] != px[i + 2]) {
                        if (diff++ == 0) first = i / 4;
                    }
                }
                return diff == 0 ? null : diff + " pixels differ (first at index " + first + ")";
            }
        }
    }

    public long messages() {
        return messages;
    }

    /** The player goes away. */
    public void close() {
        UiClients.forget(player);
    }
}
