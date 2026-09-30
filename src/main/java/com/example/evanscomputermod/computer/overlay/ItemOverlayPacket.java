package com.example.evanscomputermod.computer.overlay;

//? if <=1.21.1 {

import com.example.evanscomputermod.EvansComputerMod;
import net.minecraft.core.BlockPos;
import net.minecraft.network.RegistryFriendlyByteBuf;
import net.minecraft.network.codec.StreamCodec;
import net.minecraft.network.protocol.common.custom.CustomPacketPayload;
import net.minecraft.world.item.ItemStack;

import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;

/**
 * Server to client: the item overlay of one display of the computer at
 * {@code pos} (target 0 = terminal, 1 = Screen cluster), plus the item stacks
 * of prototypes this client hasn't been sent yet.
 */
public record ItemOverlayPacket(BlockPos pos, byte target, long generation, List<ItemOverlays.Entry> entries,
                                Map<Integer, ItemStack> protos) implements CustomPacketPayload {

    public static final Type<ItemOverlayPacket> TYPE = new Type<>(EvansComputerMod.id("item_overlay"));

    public static final StreamCodec<RegistryFriendlyByteBuf, ItemOverlayPacket> STREAM_CODEC = new StreamCodec<>() {
        @Override
        public ItemOverlayPacket decode(RegistryFriendlyByteBuf buf) {
            BlockPos pos = buf.readBlockPos();
            byte target = buf.readByte();
            long gen = buf.readVarLong();
            int n = buf.readVarInt();
            if (n < 0 || n > ItemOverlays.MAX_ENTRIES) throw new IllegalArgumentException("too many overlay entries");
            List<ItemOverlays.Entry> entries = new ArrayList<>(n);
            for (int i = 0; i < n; i++) {
                int x = buf.readShort(), y = buf.readShort(), size = buf.readUnsignedShort();
                int cx = buf.readShort(), cy = buf.readShort(), cw = buf.readUnsignedShort(), ch = buf.readUnsignedShort();
                int flags = buf.readUnsignedByte();
                int len = buf.readUnsignedByte();
                byte[] label = new byte[len];
                buf.readBytes(label);
                int proto = buf.readVarInt();
                entries.add(new ItemOverlays.Entry(x, y, size, cx, cy, cw, ch, flags,
                        new String(label, StandardCharsets.UTF_8), proto));
            }
            int p = buf.readVarInt();
            Map<Integer, ItemStack> protos = new HashMap<>();
            for (int i = 0; i < p; i++) {
                int id = buf.readVarInt();
                protos.put(id, ItemStack.STREAM_CODEC.decode(buf));
            }
            return new ItemOverlayPacket(pos, target, gen, entries, protos);
        }

        @Override
        public void encode(RegistryFriendlyByteBuf buf, ItemOverlayPacket packet) {
            buf.writeBlockPos(packet.pos());
            buf.writeByte(packet.target());
            buf.writeVarLong(packet.generation());
            buf.writeVarInt(packet.entries().size());
            for (ItemOverlays.Entry e : packet.entries()) {
                buf.writeShort(e.x());
                buf.writeShort(e.y());
                buf.writeShort(e.size());
                buf.writeShort(e.clipX());
                buf.writeShort(e.clipY());
                buf.writeShort(e.clipW());
                buf.writeShort(e.clipH());
                buf.writeByte(e.flags());
                byte[] label = e.label().getBytes(StandardCharsets.UTF_8);
                buf.writeByte(label.length);
                buf.writeBytes(label);
                buf.writeVarInt(e.proto());
            }
            buf.writeVarInt(packet.protos().size());
            for (var e : packet.protos().entrySet()) {
                buf.writeVarInt(e.getKey());
                ItemStack.STREAM_CODEC.encode(buf, e.getValue());
            }
        }
    };

    @Override
    public Type<? extends CustomPacketPayload> type() {
        return TYPE;
    }
}
//?}
