package com.example.evanscomputermod.network;

import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.client.ClientDisplayManager;
import com.example.evanscomputermod.computer.Framebuffer;
import io.netty.buffer.ByteBuf;
import net.minecraft.core.BlockPos;
import net.minecraft.network.codec.StreamCodec;
import net.minecraft.network.protocol.common.custom.CustomPacketPayload;
import net.minecraft.resources.ResourceLocation;
import net.neoforged.neoforge.network.handling.IPayloadContext;

import java.util.ArrayList;
import java.util.List;

/**
 * Packet sent from server to client containing dirty framebuffer tiles.
 */
public record FramebufferUpdatePacket(
        BlockPos pos,
        int fullWidth,
        int fullHeight,
        int tileSize,
        List<TileData> tiles
) implements CustomPacketPayload {

    public record TileData(short tileX, short tileY, short tileWidth, short tileHeight, byte[] pixelData) {}

    public static final Type<FramebufferUpdatePacket> TYPE =
            new Type<>(ResourceLocation.fromNamespaceAndPath(EvansComputerMod.MODID, "fb_update"));

    public static final StreamCodec<ByteBuf, FramebufferUpdatePacket> STREAM_CODEC = new StreamCodec<>() {
        @Override
        public FramebufferUpdatePacket decode(ByteBuf buf) {
            BlockPos pos = BlockPos.STREAM_CODEC.decode(buf);
            int fullWidth = buf.readInt();
            int fullHeight = buf.readInt();
            int tileSize = buf.readInt();
            int tileCount = buf.readInt();

            List<TileData> tiles = new ArrayList<>(tileCount);
            for (int i = 0; i < tileCount; i++) {
                short tx = buf.readShort();
                short ty = buf.readShort();
                short tw = buf.readShort();
                short th = buf.readShort();
                int dataLen = buf.readInt();
                byte[] data = new byte[dataLen];
                buf.readBytes(data);
                tiles.add(new TileData(tx, ty, tw, th, data));
            }

            return new FramebufferUpdatePacket(pos, fullWidth, fullHeight, tileSize, tiles);
        }

        @Override
        public void encode(ByteBuf buf, FramebufferUpdatePacket pkt) {
            BlockPos.STREAM_CODEC.encode(buf, pkt.pos());
            buf.writeInt(pkt.fullWidth());
            buf.writeInt(pkt.fullHeight());
            buf.writeInt(pkt.tileSize());
            buf.writeInt(pkt.tiles().size());

            for (TileData tile : pkt.tiles()) {
                buf.writeShort(tile.tileX());
                buf.writeShort(tile.tileY());
                buf.writeShort(tile.tileWidth());
                buf.writeShort(tile.tileHeight());
                buf.writeInt(tile.pixelData().length);
                buf.writeBytes(tile.pixelData());
            }
        }
    };

    @Override
    public Type<? extends CustomPacketPayload> type() {
        return TYPE;
    }

    /**
     * Creates a packet from a list of dirty tiles.
     */
    public static FramebufferUpdatePacket fromDirtyTiles(BlockPos pos, int width, int height,
                                                          int tileSize, List<Framebuffer.DirtyTile> dirtyTiles) {
        List<TileData> tiles = new ArrayList<>(dirtyTiles.size());
        for (Framebuffer.DirtyTile dt : dirtyTiles) {
            tiles.add(new TileData(
                    (short) dt.tileX(), (short) dt.tileY(),
                    (short) dt.tileWidth(), (short) dt.tileHeight(),
                    dt.pixelData()));
        }
        return new FramebufferUpdatePacket(pos, width, height, tileSize, tiles);
    }

    /**
     * Handles the packet on the client side.
     */
    public static void handle(FramebufferUpdatePacket packet, IPayloadContext context) {
        context.enqueueWork(() -> {
            ClientDisplayManager.handleUpdatePacket(packet);
        });
    }
}
