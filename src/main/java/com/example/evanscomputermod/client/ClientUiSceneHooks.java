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

    public static void handleScene(UiScenePackets.Scene packet, IPayloadContext context) {
        context.enqueueWork(() -> {
            if (Minecraft.getInstance().level == null) return;
            if (packet.stream() != 0) ClientUiScenes.bindStream(packet.stream(), packet.computerId(), packet.displayId());
            handle(packet.computerId(), packet.displayId(), packet.data());
        });
    }

    public static void handleStep(UiScenePackets.Step packet, IPayloadContext context) {
        context.enqueueWork(() -> {
            if (Minecraft.getInstance().level == null) return;
            ClientUiScenes.Key key = ClientUiScenes.stream(packet.stream());
            if (key != null) handle(key.computerId(), key.displayId(), packet.data());
        });
    }

    private static void handle(java.util.UUID computerId, int displayId, byte[] data) {
        ClientUiScenes.Ack ack = ClientUiScenes.onMessage(computerId, displayId, data, nowMs());
        if (ack != null) {
            ClientPacketDistributor.sendToServer(new UiScenePackets.Ack(computerId, (byte) displayId,
                    ack.epoch(), ack.generation(), ack.missing()));
        }
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
    }
}
