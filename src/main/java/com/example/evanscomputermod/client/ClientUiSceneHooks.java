package com.example.evanscomputermod.client;

import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.computer.ui.ClientUiScenes;
import com.example.evanscomputermod.computer.ui.UiSceneCodec;
import com.example.evanscomputermod.network.UiScenePackets;
import net.minecraft.client.Minecraft;
import net.neoforged.api.distmarker.Dist;
import net.neoforged.bus.api.SubscribeEvent;
import net.neoforged.fml.common.EventBusSubscriber;
import net.neoforged.neoforge.client.event.ClientPlayerNetworkEvent;
import net.neoforged.neoforge.client.event.RenderFrameEvent;
import net.neoforged.neoforge.client.network.ClientPacketDistributor;
import net.neoforged.neoforge.network.handling.IPayloadContext;

/**
 * Client glue of GUI client rendering (docs/gui/UI_PROTOCOL.md §14 phase
 * 2a): says hello on joining (so the server sends scenes instead of
 * pixels), applies scene messages on the main thread and acks them, and
 * draws changed mirrors once per rendered frame, which is also when window
 * transitions advance. The work itself is in {@link ClientUiScenes}.
 *
 * <p>Start the client with {@code -Decm.ui.clientRendering=false} to keep
 * the tile path (the server then treats it as a client without the
 * capability).
 */
@EventBusSubscriber(modid = EvansComputerMod.MODID, value = Dist.CLIENT)
public final class ClientUiSceneHooks {
    private ClientUiSceneHooks() {}

    public static long nowMs() {
        return System.nanoTime() / 1_000_000L;
    }

    public static boolean enabled() {
        return !"false".equals(System.getProperty("ecm.ui.clientRendering"));
    }

    /** Network thread: bind the stream id now (later steps use it), decode on the decode thread. */
    public static void handleScene(UiScenePackets.Scene packet, IPayloadContext context) {
        if (packet.stream() != 0) ClientUiScenes.bindStream(packet.stream(), packet.computerId(), packet.displayId());
        receive(packet.computerId(), packet.displayId(), packet.data());
    }

    public static void handleStep(UiScenePackets.Step packet, IPayloadContext context) {
        ClientUiScenes.Key key = ClientUiScenes.stream(packet.stream());
        if (key != null) receive(key.computerId(), key.displayId(), packet.data());
    }

    /** Per display: the epoch and generation acked on receipt (decode thread only). */
    private static final java.util.Map<ClientUiScenes.Key, long[]> RECEIVED = new java.util.concurrent.ConcurrentHashMap<>();

    /**
     * Unpack (inflate, decode) on the decode thread and ack on receipt
     * (DISPLAY_PIPELINE_SPEC.md Part A5) when the message continues the
     * stream: a keyframe, or the step after the last one acked. The render
     * thread applies it and only answers when something went wrong (a
     * keyframe request, missing images).
     */
    private static void receive(java.util.UUID computerId, int displayId, byte[] data) {
        long recvUs = com.example.evanscomputermod.network.ClockSync.clientNowUs();
        DisplayClientHooks.decode(() -> {
            if (Minecraft.getInstance().level == null) return;
            UiSceneCodec.Message m;
            try {
                m = UiSceneCodec.unpack(data);
            } catch (com.example.evanscomputermod.computer.ui.proto.UiProtoException e) {
                m = null;
            }
            ClientUiScenes.Key key = new ClientUiScenes.Key(computerId, displayId);
            long acked = -1;
            if (m != null && m.patch() != null) {
                long[] r = RECEIVED.get(key);
                if (m.kind() == UiSceneCodec.KEYFRAME) {
                    RECEIVED.put(key, new long[]{m.epoch(), m.patch().to()});
                    acked = m.patch().to();
                    send(computerId, displayId, m.epoch(), acked, null, recvUs);
                } else if (m.kind() == UiSceneCodec.STEP && r != null && r[1] == m.patch().from()) {
                    r[1] = m.patch().to();
                    acked = r[1];
                    send(computerId, displayId, (int) r[0], acked, null, recvUs);
                }
            } else if (m != null && m.kind() == UiSceneCodec.END) {
                RECEIVED.remove(key);
            } else if (m != null && m.timed() != null) {
                // A timed frame (protocol v2): decoded here, acked on receipt with its own sequence.
                if (m.kind() == UiSceneCodec.TIMED) m.timed().decode();
                send(computerId, displayId, m.epoch(),
                        com.example.evanscomputermod.computer.ui.UiSceneHub.TIMED_ACK | m.timed().tseq(), null, recvUs);
            }
            UiSceneCodec.Message msg = m;
            long already = acked;
            DisplayClientHooks.toMain(() -> {
                ClientUiScenes.Ack ack = ClientUiScenes.onMessage(computerId, displayId, msg, data.length, nowMs(), recvUs);
                if (ack == null) return;
                if (ack.generation() <= 0 || ack.missing() != null) RECEIVED.remove(key);
                else if (already >= 0 && ack.generation() <= already) return; // acked on receipt
                send(computerId, displayId, ack.epoch(), ack.generation(), ack.missing(), recvUs);
            });
        });
    }

    private static void send(java.util.UUID computerId, int displayId, int epoch, long gen, long[] missing, long recvUs) {
        ClientPacketDistributor.sendToServer(new UiScenePackets.Ack(computerId, (byte) displayId, epoch, gen, missing, recvUs));
    }

    @SubscribeEvent
    public static void onFrame(RenderFrameEvent.Pre event) {
        if (Minecraft.getInstance().level == null) return;
        ClientUiScenes.tick(nowMs());
    }

    @SubscribeEvent
    public static void onLoggingIn(ClientPlayerNetworkEvent.LoggingIn event) {
        if (!enabled()) return;
        ClientPacketDistributor.sendToServer(new UiScenePackets.Hello(UiSceneCodec.VERSION, ClientUiScenes.heldImageKeys()));
    }

    @SubscribeEvent
    public static void onLoggingOut(ClientPlayerNetworkEvent.LoggingOut event) {
        ClientUiScenes.clearScenes();
        RECEIVED.clear();
    }
}
