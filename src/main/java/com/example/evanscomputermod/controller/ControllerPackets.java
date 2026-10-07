package com.example.evanscomputermod.controller;

import com.example.evanscomputermod.EvansComputerMod;
import io.netty.buffer.ByteBuf;
import net.minecraft.network.codec.ByteBufCodecs;
import net.minecraft.network.codec.StreamCodec;
import net.minecraft.network.protocol.common.custom.CustomPacketPayload;
import net.minecraft.resources.Identifier;
import net.minecraft.world.InteractionHand;

import java.util.EnumMap;
import java.util.Map;
import java.util.UUID;

/** Network messages of the Wireless Controller. */
public final class ControllerPackets {

    private ControllerPackets() {}

    private static UUID readUuid(ByteBuf buf) {
        return new UUID(buf.readLong(), buf.readLong());
    }

    private static void writeUuid(ByteBuf buf, UUID id) {
        buf.writeLong(id.getMostSignificantBits());
        buf.writeLong(id.getLeastSignificantBits());
    }

    /**
     * Client to server: the controller's current state, sent on every change and
     * at least every 0.5 s while connected. {@code connected = false} disconnects.
     * Only the controller's id is trusted from the client; which computer it
     * talks to comes from the item in the player's inventory.
     */
    public record Input(UUID controllerId, boolean connected, ControllerState state) implements CustomPacketPayload {
        public static final Type<Input> TYPE =
                new Type<>(Identifier.fromNamespaceAndPath(EvansComputerMod.MODID, "controller_input"));

        public static final StreamCodec<ByteBuf, Input> STREAM_CODEC = new StreamCodec<>() {
            @Override
            public Input decode(ByteBuf buf) {
                UUID id = readUuid(buf);
                boolean connected = buf.readBoolean();
                int buttons = buf.readUnsignedShort();
                int lx = buf.readByte();
                int ly = buf.readByte();
                int rx = buf.readByte();
                int ry = buf.readByte();
                int lt = buf.readUnsignedByte();
                int rt = buf.readUnsignedByte();
                return new Input(id, connected, new ControllerState(buttons, lx, ly, rx, ry, lt, rt));
            }

            @Override
            public void encode(ByteBuf buf, Input p) {
                writeUuid(buf, p.controllerId());
                buf.writeBoolean(p.connected());
                ControllerState s = p.state();
                buf.writeShort(s.buttons());
                buf.writeByte(s.lx());
                buf.writeByte(s.ly());
                buf.writeByte(s.rx());
                buf.writeByte(s.ry());
                buf.writeByte(s.lt());
                buf.writeByte(s.rt());
            }
        };

        @Override
        public Type<? extends CustomPacketPayload> type() {
            return TYPE;
        }
    }

    /** Client to server: new key bindings for the controller in {@code hand}. */
    public record Bindings(InteractionHand hand, Map<ControllerInput, String> bindings) implements CustomPacketPayload {
        public static final Type<Bindings> TYPE =
                new Type<>(Identifier.fromNamespaceAndPath(EvansComputerMod.MODID, "controller_bindings"));

        private static final StreamCodec<ByteBuf, String> KEY_NAME = ByteBufCodecs.stringUtf8(ControllerData.MAX_KEY_NAME);

        public static final StreamCodec<ByteBuf, Bindings> STREAM_CODEC = new StreamCodec<>() {
            @Override
            public Bindings decode(ByteBuf buf) {
                InteractionHand hand = buf.readBoolean() ? InteractionHand.OFF_HAND : InteractionHand.MAIN_HAND;
                int n = buf.readUnsignedByte();
                Map<ControllerInput, String> map = new EnumMap<>(ControllerInput.class);
                for (int i = 0; i < n; i++) {
                    int ordinal = buf.readUnsignedByte();
                    String key = KEY_NAME.decode(buf);
                    if (ordinal < ControllerInput.values().length) map.put(ControllerInput.values()[ordinal], key);
                }
                return new Bindings(hand, map);
            }

            @Override
            public void encode(ByteBuf buf, Bindings p) {
                buf.writeBoolean(p.hand() == InteractionHand.OFF_HAND);
                buf.writeByte(p.bindings().size());
                for (Map.Entry<ControllerInput, String> e : p.bindings().entrySet()) {
                    buf.writeByte(e.getKey().ordinal());
                    KEY_NAME.encode(buf, e.getValue());
                }
            }
        };

        @Override
        public Type<? extends CustomPacketPayload> type() {
            return TYPE;
        }
    }

    /**
     * Server to client: whether a controller is connected, as which player
     * (1-4, 0 = not connected), and why not.
     */
    public record Status(UUID controllerId, int player, String message) implements CustomPacketPayload {
        public static final Type<Status> TYPE =
                new Type<>(Identifier.fromNamespaceAndPath(EvansComputerMod.MODID, "controller_status"));

        private static final StreamCodec<ByteBuf, String> MESSAGE = ByteBufCodecs.stringUtf8(256);

        public static final StreamCodec<ByteBuf, Status> STREAM_CODEC = new StreamCodec<>() {
            @Override
            public Status decode(ByteBuf buf) {
                return new Status(readUuid(buf), buf.readUnsignedByte(), MESSAGE.decode(buf));
            }

            @Override
            public void encode(ByteBuf buf, Status p) {
                writeUuid(buf, p.controllerId());
                buf.writeByte(p.player());
                MESSAGE.encode(buf, p.message());
            }
        };

        @Override
        public Type<? extends CustomPacketPayload> type() {
            return TYPE;
        }
    }
}
