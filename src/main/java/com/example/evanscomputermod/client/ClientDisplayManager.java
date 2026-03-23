package com.example.evanscomputermod.client;

import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.config.DisplayConfig;
import com.example.evanscomputermod.network.FramebufferFullPacket;
import com.example.evanscomputermod.network.FramebufferUpdatePacket;
import com.mojang.blaze3d.platform.NativeImage;
import net.minecraft.client.Minecraft;
import net.minecraft.client.renderer.texture.DynamicTexture;
import net.minecraft.core.BlockPos;
import net.minecraft.resources.ResourceLocation;

import javax.annotation.Nullable;
import java.util.Map;
import java.util.concurrent.ConcurrentHashMap;

/**
 * Client-side manager for display block textures.
 * Manages NativeImage + DynamicTexture instances per display block.
 */
public class ClientDisplayManager {

    private static final Map<BlockPos, DisplayClientState> displays = new ConcurrentHashMap<>();

    /**
     * State for a single client-side display.
     */
    public static class DisplayClientState {
        public NativeImage image;
        public DynamicTexture texture;
        public ResourceLocation textureId;
        public int width;
        public int height;
        public boolean dirty;

        public DisplayClientState(int width, int height) {
            this.width = width;
            this.height = height;
            this.image = new NativeImage(NativeImage.Format.RGBA, width, height, false);
            // Initialize to black
            for (int y = 0; y < height; y++) {
                for (int x = 0; x < width; x++) {
                    image.setPixelRGBA(x, y, 0xFF000000); // ABGR format: fully opaque black
                }
            }
            this.texture = new DynamicTexture(image);
            this.textureId = Minecraft.getInstance().getTextureManager()
                    .register("evanscomputermod_display", this.texture);
            this.dirty = false;
        }

        public void close() {
            texture.close();
            // NativeImage is closed by DynamicTexture
        }
    }

    /**
     * Gets the display state for a given position, or null if not tracked.
     */
    @Nullable
    public static DisplayClientState getDisplay(BlockPos pos) {
        return displays.get(pos);
    }

    /**
     * Handles a tile update packet from the server.
     */
    public static void handleUpdatePacket(FramebufferUpdatePacket packet) {
        DisplayClientState state = displays.get(packet.pos());

        // Create state if needed
        if (state == null) {
            int maxTextures = DisplayConfig.COMMON_SPEC.isLoaded()
                    ? DisplayConfig.COMMON.maxClientTextures.get() : 32;
            if (displays.size() >= maxTextures) {
                return; // Over limit
            }
            state = new DisplayClientState(packet.fullWidth(), packet.fullHeight());
            displays.put(packet.pos(), state);
        }

        // Resize if dimensions changed
        if (state.width != packet.fullWidth() || state.height != packet.fullHeight()) {
            state.close();
            state = new DisplayClientState(packet.fullWidth(), packet.fullHeight());
            displays.put(packet.pos(), state);
        }

        // Apply tile updates
        applyTilesFromPacket(state, packet);
    }

    /**
     * Handles a full framebuffer packet from the server.
     */
    public static void handleFullPacket(FramebufferFullPacket packet) {
        DisplayClientState state = displays.get(packet.pos());

        if (state == null || state.width != packet.width() || state.height != packet.height()) {
            if (state != null) state.close();
            int maxTextures = DisplayConfig.COMMON_SPEC.isLoaded()
                    ? DisplayConfig.COMMON.maxClientTextures.get() : 32;
            if (displays.size() >= maxTextures && state == null) {
                return;
            }
            state = new DisplayClientState(packet.width(), packet.height());
            displays.put(packet.pos(), state);
        }

        byte[] pixels = packet.decompressPixels();
        int w = state.width;
        int h = state.height;

        for (int y = 0; y < h; y++) {
            for (int x = 0; x < w; x++) {
                int idx = (y * w + x) * 4;
                if (idx + 3 < pixels.length) {
                    int r = pixels[idx] & 0xFF;
                    int g = pixels[idx + 1] & 0xFF;
                    int b = pixels[idx + 2] & 0xFF;
                    int a = pixels[idx + 3] & 0xFF;
                    // NativeImage uses ABGR format
                    state.image.setPixelRGBA(x, y, (a << 24) | (b << 16) | (g << 8) | r);
                }
            }
        }

        state.texture.upload();
        state.dirty = false;
    }

    /**
     * Removes a display from tracking (e.g., chunk unload).
     */
    public static void removeDisplay(BlockPos pos) {
        DisplayClientState state = displays.remove(pos);
        if (state != null) {
            state.close();
        }
    }

    /**
     * Removes all tracked displays.
     */
    public static void clear() {
        displays.values().forEach(DisplayClientState::close);
        displays.clear();
    }

    /**
     * Applies a single tile's pixel data to the NativeImage.
     */
    private static void applyTileToImage(NativeImage image, FramebufferUpdatePacket.TileData tile) {
        int tilePixelX = tile.tileX() * 16; // tile coords are in tile units; we use pixelX from data
        // Actually, we need to compute from tile coords and known tile size.
        // But the tile data includes tileWidth and tileHeight, so we reconstruct:
        int tw = tile.tileWidth();
        int th = tile.tileHeight();
        // Tile position in pixels = tileX * tileSize, but we don't have tileSize here.
        // We'll compute from the tile grid coordinates and the actual pixel data dimensions.
        // For the update packet: tileX/tileY are grid coordinates. We need to know tileSize.
        // The packet includes it at the top level - but inside this method we don't have it.
        // Let's just use the tile data directly with the grid coords.

        // Actually, let's fix this by computing pixel offsets from the tile coordinates
        // We need the tile size from the packet - let's pass it through.
        // For now, reconstruct from the tile data dimensions (works for interior tiles,
        // edge tiles are smaller but tileX * some_size wouldn't help without the size).

        // We'll need to use pixelX/pixelY. Let me refactor to pass tileSize.
        // For now, use a simple approach: tile pixel position = tileX * (pixel data width for full tile)
        // This won't work for edge tiles. Let me fix this properly.

        // The simplest fix: store pixel offsets in TileData. But we already have tileX/tileY as shorts.
        // Let's just pass the tileSize through.
        // For now: skip this method and do it inline in handleUpdatePacket.
    }

    /**
     * Applies tile updates with known tile size.
     */
    private static void applyTileToImage(NativeImage image, FramebufferUpdatePacket.TileData tile, int tileSize) {
        int pixelX = tile.tileX() * tileSize;
        int pixelY = tile.tileY() * tileSize;
        int tw = tile.tileWidth();
        int th = tile.tileHeight();
        byte[] data = tile.pixelData();

        for (int row = 0; row < th; row++) {
            for (int col = 0; col < tw; col++) {
                int idx = (row * tw + col) * 4;
                if (idx + 3 >= data.length) continue;

                int x = pixelX + col;
                int y = pixelY + row;
                if (x >= image.getWidth() || y >= image.getHeight()) continue;

                int r = data[idx] & 0xFF;
                int g = data[idx + 1] & 0xFF;
                int b = data[idx + 2] & 0xFF;
                int a = data[idx + 3] & 0xFF;
                // NativeImage uses ABGR format
                image.setPixelRGBA(x, y, (a << 24) | (b << 16) | (g << 8) | r);
            }
        }
    }

    /**
     * Handles a tile update packet with tile size awareness.
     */
    static void applyTilesFromPacket(DisplayClientState state, FramebufferUpdatePacket packet) {
        for (FramebufferUpdatePacket.TileData tile : packet.tiles()) {
            applyTileToImage(state.image, tile, packet.tileSize());
        }
        state.texture.upload();
    }
}
