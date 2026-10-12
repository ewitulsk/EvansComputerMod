package com.example.evanscomputermod.phone;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
import io.netty.buffer.ByteBuf;
import net.minecraft.core.UUIDUtil;
import net.minecraft.network.codec.ByteBufCodecs;
import net.minecraft.network.codec.StreamCodec;
import net.minecraft.network.protocol.common.custom.CustomPacketPayload;
import net.minecraft.resources.ResourceLocation;

import java.util.UUID;

/**
 * ePhone payloads. Display frames and open-mode keyboard/pointer input use the
 * computer's ordinary UUID-addressed packets ({@code TerminalDeltaPacket},
 * {@code TerminalReadyPacket}, {@code TerminalInputPacket}, {@code MouseInputPacket});
 * these are the phone-only extras.
 */
public final class PhonePackets {
    private PhonePackets() {}

    private static ResourceLocation id(String path) {
        return ResourceLocation.fromNamespaceAndPath(EvansComputerMod.MODID, path);
    }

    /**
     * Client to server: an in-hand hotkey of the phone {@code phone} the player
     * holds. {@code slot} 1..6 are the hotkey slots (down and up); 0 is the
     * OS "switch app" key (down only).
     */
    public record Hotkey(UUID phone, int slot, boolean down) implements CustomPacketPayload {
        public static final Type<Hotkey> TYPE = new Type<>(id("ephone_hotkey"));
        public static final StreamCodec<ByteBuf, Hotkey> STREAM_CODEC = StreamCodec.composite(
                UUIDUtil.STREAM_CODEC, Hotkey::phone, ByteBufCodecs.VAR_INT, Hotkey::slot, ByteBufCodecs.BOOL, Hotkey::down,
                Hotkey::new);

        @Override
        public Type<? extends CustomPacketPayload> type() {
            return TYPE;
        }
    }

    /** Client to server: the player's visibility choices (PhoneClientConfig). */
    public record Prefs(boolean share, boolean renderOthers) implements CustomPacketPayload {
        public static final Type<Prefs> TYPE = new Type<>(id("ephone_prefs"));
        public static final StreamCodec<ByteBuf, Prefs> STREAM_CODEC = StreamCodec.composite(
                ByteBufCodecs.BOOL, Prefs::share, ByteBufCodecs.BOOL, Prefs::renderOthers, Prefs::new);

        public static final Prefs DEFAULT = new Prefs(true, true);

        @Override
        public Type<? extends CustomPacketPayload> type() {
            return TYPE;
        }
    }

    /**
     * Server to client: a phone's state ({@link PhoneState} ordinal), what
     * onlookers may see ({@link PhoneConfig.Visibility} ordinal) and its charge
     * in hundredths of a percent. Sent to the holder and players near it.
     */
    public record Status(UUID phone, int state, int visibility, int batteryCenti) implements CustomPacketPayload {
        public static final Type<Status> TYPE = new Type<>(id("ephone_status"));
        public static final StreamCodec<ByteBuf, Status> STREAM_CODEC = StreamCodec.composite(
                UUIDUtil.STREAM_CODEC, Status::phone, ByteBufCodecs.VAR_INT, Status::state,
                ByteBufCodecs.VAR_INT, Status::visibility, ByteBufCodecs.VAR_INT, Status::batteryCenti, Status::new);

        public PhoneState phoneState() {
            return PhoneState.values()[Math.floorMod(state, PhoneState.values().length)];
        }

        public PhoneConfig.Visibility phoneVisibility() {
            return PhoneConfig.Visibility.values()[Math.floorMod(visibility, PhoneConfig.Visibility.values().length)];
        }

        @Override
        public Type<? extends CustomPacketPayload> type() {
            return TYPE;
        }
    }
}
//?}
