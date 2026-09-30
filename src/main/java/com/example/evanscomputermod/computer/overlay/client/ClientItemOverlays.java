package com.example.evanscomputermod.computer.overlay.client;

//? if <=1.21.1 {

import com.example.evanscomputermod.computer.overlay.ItemOverlayPacket;
import com.example.evanscomputermod.computer.overlay.ItemOverlays;
import net.minecraft.core.BlockPos;
import net.minecraft.world.item.ItemStack;
import net.neoforged.neoforge.client.event.ClientPlayerNetworkEvent;
import net.neoforged.neoforge.client.event.ClientTickEvent;
import net.neoforged.neoforge.common.NeoForge;
import net.neoforged.neoforge.network.handling.IPayloadContext;
import org.jetbrains.annotations.Nullable;

import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.concurrent.ConcurrentHashMap;

/** Client copy of every computer display's item overlay, keyed by the computer's position and display. */
public final class ClientItemOverlays {

    public record Key(BlockPos pos, int target) {
    }

    public static final class Overlay {
        public volatile List<ItemOverlays.Entry> entries = List.of();
        final Map<Integer, ItemStack> protos = new HashMap<>();

        @Nullable
        public ItemStack stack(ItemOverlays.Entry e) {
            return protos.get(e.proto());
        }
    }

    private static final Map<Key, Overlay> OVERLAYS = new ConcurrentHashMap<>();

    private ClientItemOverlays() {
    }

    public static void register() {
        NeoForge.EVENT_BUS.addListener((ClientPlayerNetworkEvent.LoggingOut e) -> OVERLAYS.clear());
        NeoForge.EVENT_BUS.addListener((ClientTickEvent.Post e) -> OverlayHover.tick());
    }

    public static void handle(ItemOverlayPacket packet, IPayloadContext ctx) {
        ctx.enqueueWork(() -> {
            Overlay o = OVERLAYS.computeIfAbsent(new Key(packet.pos(), packet.target()), k -> new Overlay());
            o.protos.putAll(packet.protos());
            o.entries = packet.entries();
        });
    }

    @Nullable
    public static Overlay get(@Nullable BlockPos pos, int target) {
        if (pos == null) return null;
        Overlay o = OVERLAYS.get(new Key(pos, target));
        return o == null || o.entries.isEmpty() ? null : o;
    }
}
//?}
