package com.example.evanscomputermod.network;

import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.block.MissileLauncherBlockEntity;
import io.netty.buffer.ByteBuf;
import net.minecraft.client.Minecraft;
import net.minecraft.core.BlockPos;
import net.minecraft.network.codec.ByteBufCodecs;
import net.minecraft.network.codec.StreamCodec;
import net.minecraft.network.protocol.common.custom.CustomPacketPayload;
import net.minecraft.resources.Identifier;
import net.neoforged.api.distmarker.Dist;
import net.neoforged.api.distmarker.OnlyIn;

/**
 * Server → client: syncs a launcher's bearing and elevation so the barrel
 * renderer stays in sync with Python-set values.
 *
 * Payload: BlockPos (8 bytes) + bearing float (4 bytes) + elevation float (4 bytes) = 16 bytes.
 */
public record LauncherStatePacket(
        BlockPos pos,
        float bearing,
        float elevation
) implements CustomPacketPayload {

    public static final Type<LauncherStatePacket> TYPE =
            new Type<>(Identifier.fromNamespaceAndPath(EvansComputerMod.MODID, "launcher_state"));

    public static final StreamCodec<ByteBuf, LauncherStatePacket> STREAM_CODEC = StreamCodec.composite(
            BlockPos.STREAM_CODEC,        LauncherStatePacket::pos,
            ByteBufCodecs.FLOAT,          LauncherStatePacket::bearing,
            ByteBufCodecs.FLOAT,          LauncherStatePacket::elevation,
            LauncherStatePacket::new
    );

    @Override
    public Type<? extends CustomPacketPayload> type() {
        return TYPE;
    }

    /** Client-side handler: update the block entity's visual bearing/elevation. */
    @OnlyIn(Dist.CLIENT)
    public static void handleOnClient(LauncherStatePacket packet) {
        Minecraft mc = Minecraft.getInstance();
        if (mc.level == null) return;
        if (mc.level.getBlockEntity(packet.pos()) instanceof MissileLauncherBlockEntity launcher) {
            launcher.setClientBearing(packet.bearing());
            launcher.setClientElevation(packet.elevation());
        }
    }
}
