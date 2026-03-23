package com.example.evanscomputermod.network;

import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.client.ClientDisplayManager;
import io.netty.buffer.ByteBuf;
import net.minecraft.core.BlockPos;
import net.minecraft.network.codec.StreamCodec;
import net.minecraft.network.protocol.common.custom.CustomPacketPayload;
import net.minecraft.resources.ResourceLocation;
import net.neoforged.neoforge.network.handling.IPayloadContext;

import java.io.ByteArrayInputStream;
import java.io.ByteArrayOutputStream;
import java.util.zip.DeflaterOutputStream;
import java.util.zip.InflaterInputStream;

/**
 * Packet sent from server to client containing the full framebuffer.
 * Used when a player first starts tracking a display chunk.
 */
public record FramebufferFullPacket(
        BlockPos pos,
        int width,
        int height,
        byte[] compressedPixels
) implements CustomPacketPayload {

    public static final Type<FramebufferFullPacket> TYPE =
            new Type<>(ResourceLocation.fromNamespaceAndPath(EvansComputerMod.MODID, "fb_full"));

    public static final StreamCodec<ByteBuf, FramebufferFullPacket> STREAM_CODEC = new StreamCodec<>() {
        @Override
        public FramebufferFullPacket decode(ByteBuf buf) {
            BlockPos pos = BlockPos.STREAM_CODEC.decode(buf);
            int width = buf.readInt();
            int height = buf.readInt();
            int dataLen = buf.readInt();
            byte[] data = new byte[dataLen];
            buf.readBytes(data);
            return new FramebufferFullPacket(pos, width, height, data);
        }

        @Override
        public void encode(ByteBuf buf, FramebufferFullPacket pkt) {
            BlockPos.STREAM_CODEC.encode(buf, pkt.pos());
            buf.writeInt(pkt.width());
            buf.writeInt(pkt.height());
            buf.writeInt(pkt.compressedPixels().length);
            buf.writeBytes(pkt.compressedPixels());
        }
    };

    @Override
    public Type<? extends CustomPacketPayload> type() {
        return TYPE;
    }

    /**
     * Creates a packet from raw RGBA pixel data, optionally compressing.
     */
    public static FramebufferFullPacket fromPixels(BlockPos pos, int width, int height,
                                                    byte[] pixels, boolean compress) {
        if (compress) {
            try {
                ByteArrayOutputStream baos = new ByteArrayOutputStream();
                DeflaterOutputStream dos = new DeflaterOutputStream(baos);
                dos.write(pixels);
                dos.close();
                return new FramebufferFullPacket(pos, width, height, baos.toByteArray());
            } catch (Exception e) {
                EvansComputerMod.LOGGER.warn("Failed to compress framebuffer, sending raw", e);
            }
        }
        return new FramebufferFullPacket(pos, width, height, pixels.clone());
    }

    /**
     * Decompresses the pixel data.
     */
    public byte[] decompressPixels() {
        int expectedSize = width * height * 4;
        // Check if it's compressed (compressed data will be smaller)
        if (compressedPixels.length < expectedSize) {
            try {
                ByteArrayInputStream bais = new ByteArrayInputStream(compressedPixels);
                InflaterInputStream iis = new InflaterInputStream(bais);
                byte[] result = iis.readAllBytes();
                iis.close();
                return result;
            } catch (Exception e) {
                EvansComputerMod.LOGGER.warn("Failed to decompress framebuffer", e);
                return new byte[expectedSize];
            }
        }
        return compressedPixels;
    }

    public static void handle(FramebufferFullPacket packet, IPayloadContext context) {
        context.enqueueWork(() -> {
            ClientDisplayManager.handleFullPacket(packet);
        });
    }
}
