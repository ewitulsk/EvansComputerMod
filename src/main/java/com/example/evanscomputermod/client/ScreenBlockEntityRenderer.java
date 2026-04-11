package com.example.evanscomputermod.client;

import com.example.evanscomputermod.block.ScreenBlock;
import com.example.evanscomputermod.block.ScreenBlockEntity;
import com.example.evanscomputermod.computer.TerminalDisplay;
import com.mojang.blaze3d.vertex.PoseStack;
import com.mojang.blaze3d.vertex.VertexConsumer;
import net.minecraft.client.Minecraft;
import net.minecraft.client.renderer.SubmitNodeCollector;
import net.minecraft.client.renderer.blockentity.BlockEntityRenderer;
import net.minecraft.client.renderer.blockentity.BlockEntityRendererProvider;
import net.minecraft.client.renderer.blockentity.state.BlockEntityRenderState;
import net.minecraft.client.renderer.feature.ModelFeatureRenderer;
import net.minecraft.client.renderer.rendertype.RenderType;
import net.minecraft.client.renderer.rendertype.RenderTypes;
import net.minecraft.client.renderer.state.level.CameraRenderState;
import net.minecraft.client.renderer.texture.OverlayTexture;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.resources.Identifier;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.phys.AABB;
import net.minecraft.world.phys.Vec3;

import java.util.HashMap;
import java.util.Iterator;
import java.util.Map;

import org.jspecify.annotations.Nullable;

/**
 * Renders a screen cluster as a single textured quad on the anchor block's
 * display face, spanning the whole rectangle in world space. Non-anchor
 * screen blocks render as plain block models.
 *
 * <p>MC 26.1 BER API: uses a state class that captures per-frame data, then
 * {@link #submit} enqueues custom geometry via {@link SubmitNodeCollector}.
 */
public class ScreenBlockEntityRenderer implements BlockEntityRenderer<ScreenBlockEntity, ScreenBlockEntityRenderer.State> {

    /** Per-frame state for a screen cluster anchor. */
    public static class State extends BlockEntityRenderState {
        public boolean anchor;
        public int cols;
        public int rows;
        public int gfxW;
        public int gfxH;
        @Nullable public Direction facing;
        @Nullable public Identifier texture;
    }

    /** Shared per-anchor GPU texture cache. Keyed by block position. */
    private static final Map<BlockPos, TerminalGraphicsTexture> TEXTURES = new HashMap<>();
    /** Last dirty counters uploaded, per anchor. */
    private static final Map<BlockPos, long[]> LAST_DIRTY = new HashMap<>();

    public ScreenBlockEntityRenderer(BlockEntityRendererProvider.Context context) {
    }

    @Override
    public State createRenderState() {
        return new State();
    }

    @Override
    public int getViewDistance() {
        return 128;
    }

    /**
     * Override the default single-block AABB so that frustum culling knows
     * the anchor's BER quad extends across the whole cluster. Without this,
     * looking at a multi-block cluster from an angle where the anchor is
     * outside the view frustum causes MC to skip the BER entirely — and
     * since only the anchor draws the spanning quad, the non-anchor
     * members show no content at all.
     * <p>
     * Mirrors the axis conventions used in {@link #submit}: the anchor is
     * the cluster's viewer top-left, and the cluster extends in the
     * viewer's right direction for {@code cols} blocks and downward for
     * {@code rows} blocks.
     */
    @Override
    public AABB getRenderBoundingBox(ScreenBlockEntity be) {
        if (!be.isAnchor()) {
            // Non-anchor members don't render anything themselves; a
            // single-block AABB is fine (and cheap to cull).
            BlockPos p = be.getBlockPos();
            return new AABB(p.getX(), p.getY(), p.getZ(),
                    p.getX() + 1, p.getY() + 1, p.getZ() + 1);
        }
        int cols = be.getClusterCols();
        int rows = be.getClusterRows();
        if (cols <= 0 || rows <= 0) {
            BlockPos p = be.getBlockPos();
            return new AABB(p.getX(), p.getY(), p.getZ(),
                    p.getX() + 1, p.getY() + 1, p.getZ() + 1);
        }

        BlockState bs = be.getBlockState();
        if (!(bs.getBlock() instanceof ScreenBlock)) {
            BlockPos p = be.getBlockPos();
            return new AABB(p.getX(), p.getY(), p.getZ(),
                    p.getX() + 1, p.getY() + 1, p.getZ() + 1);
        }
        Direction facing = bs.getValue(ScreenBlock.FACING);
        BlockPos p = be.getBlockPos();

        // Start from the anchor's own 1x1x1 bounds then expand in the
        // cluster's "rightward" direction by (cols - 1) blocks and
        // downward by (rows - 1) blocks.
        double minX = p.getX();
        double minY = p.getY();
        double minZ = p.getZ();
        double maxX = p.getX() + 1;
        double maxY = p.getY() + 1;
        double maxZ = p.getZ() + 1;

        int du = cols - 1;
        int dv = rows - 1;

        switch (facing) {
            case NORTH -> {
                // viewer's right = WEST (-x). Extend -x by du.
                minX -= du;
                minY -= dv;
            }
            case SOUTH -> {
                // viewer's right = EAST (+x). Extend +x by du.
                maxX += du;
                minY -= dv;
            }
            case WEST -> {
                // viewer's right = SOUTH (+z). Extend +z by du.
                maxZ += du;
                minY -= dv;
            }
            case EAST -> {
                // viewer's right = NORTH (-z). Extend -z by du.
                minZ -= du;
                minY -= dv;
            }
            default -> {}
        }
        return new AABB(minX, minY, minZ, maxX, maxY, maxZ);
    }

    @Override
    public void extractRenderState(ScreenBlockEntity be, State state, float partialTicks, Vec3 cameraPosition,
                                    ModelFeatureRenderer.@Nullable CrumblingOverlay breakProgress) {
        BlockEntityRenderState.extractBase(be, state, breakProgress);

        state.anchor = be.isAnchor();
        state.cols = be.getClusterCols();
        state.rows = be.getClusterRows();
        state.texture = null;
        state.facing = null;
        state.gfxW = 0;
        state.gfxH = 0;

        if (!state.anchor || state.cols <= 0 || state.rows <= 0) return;

        BlockState bs = be.getBlockState();
        if (!(bs.getBlock() instanceof ScreenBlock)) return;
        // Gate on the ACTIVE blockstate — the server sets this true only
        // when the cluster is both physically valid AND powered on. A
        // cluster that's valid but powered off stays visually "off"
        // (inactive face texture, no BER quad).
        if (!bs.getValue(ScreenBlock.ACTIVE)) return;
        state.facing = bs.getValue(ScreenBlock.FACING);

        TerminalDisplay display = be.clientDisplay;
        if (display == null) return;
        int gfxW = display.getGfxWidth();
        int gfxH = display.getGfxHeight();
        if (gfxW <= 0 || gfxH <= 0) return;
        byte[] pixels = display.getPixelData();
        int[] palette = display.getPalette();
        if (pixels == null || palette == null) return;

        state.gfxW = gfxW;
        state.gfxH = gfxH;

        BlockPos anchorPos = be.getBlockPos();
        TerminalGraphicsTexture tex = TEXTURES.get(anchorPos);
        if (tex == null) {
            tex = new TerminalGraphicsTexture(gfxW, gfxH);
            TEXTURES.put(anchorPos, tex);
            LAST_DIRTY.put(anchorPos, new long[] { -1L, -1L });
        } else if (tex.getWidth() != gfxW || tex.getHeight() != gfxH) {
            tex.resize(gfxW, gfxH);
            LAST_DIRTY.put(anchorPos, new long[] { -1L, -1L });
        }

        long[] lastDirty = LAST_DIRTY.computeIfAbsent(anchorPos, k -> new long[] { -1L, -1L });
        int pixelDirty = display.getPixelDirtyCounter();
        int paletteDirty = display.getPaletteDirtyCounter();
        if (lastDirty[0] != pixelDirty || lastDirty[1] != paletteDirty) {
            tex.updateFull(pixels, palette);
            lastDirty[0] = pixelDirty;
            lastDirty[1] = paletteDirty;
        }
        state.texture = tex.getTextureId();

        if ((System.nanoTime() & 0x7FFFFFFF) == 0) pruneStaleTextures();
    }

    @Override
    public void submit(State state, PoseStack poseStack, SubmitNodeCollector submitNodeCollector, CameraRenderState camera) {
        if (!state.anchor || state.texture == null || state.facing == null) return;
        if (state.cols <= 0 || state.rows <= 0) return;

        Direction facing = state.facing;
        int cols = state.cols;
        int rows = state.rows;

        // Anchor's top-left corner of the display face (in local block coords)
        // and the in-plane "right" and "down" unit vectors. Matches the axes
        // used by ScreenClusterDiscovery: from outside the display, u→right,
        // v→up, so the rectangle extends +uAxis for `cols` and downward for
        // `rows` from the anchor's top-left.
        float eps = 0.001f;
        float tlx, tly, tlz;
        float uDx, uDy, uDz;
        float vDx = 0, vDy = -1, vDz = 0;

        // Anchor's viewer-top-left corner of the display face (in the
        // anchor block's local coords), plus the unit vector in the
        // viewer's RIGHT direction. Matches ScreenClusterDiscovery's
        // uAxisPos table (right = forward × up, forward = -normal).
        switch (facing) {
            case NORTH -> {
                // Normal -z; viewer looks +z; right = WEST (-x).
                // Top-left of face (from viewer) = east edge + top.
                tlx = 1f; tly = 1f; tlz = -eps;
                uDx = -1; uDy = 0; uDz = 0;
            }
            case SOUTH -> {
                // Normal +z; viewer looks -z; right = EAST (+x).
                // Top-left = west edge + top.
                tlx = 0f; tly = 1f; tlz = 1f + eps;
                uDx = 1; uDy = 0; uDz = 0;
            }
            case WEST -> {
                // Normal -x; viewer looks +x; right = SOUTH (+z).
                // Top-left = north edge + top.
                tlx = -eps; tly = 1f; tlz = 0f;
                uDx = 0; uDy = 0; uDz = 1;
            }
            case EAST -> {
                // Normal +x; viewer looks -x; right = NORTH (-z).
                // Top-left = south edge + top.
                tlx = 1f + eps; tly = 1f; tlz = 1f;
                uDx = 0; uDy = 0; uDz = -1;
            }
            default -> { return; }
        }

        final float tlX = tlx;
        final float tlY = tly;
        final float tlZ = tlz;
        final float trX = tlx + uDx * cols;
        final float trY = tly + uDy * cols;
        final float trZ = tlz + uDz * cols;
        final float brX = tlx + uDx * cols + vDx * rows;
        final float brY = tly + uDy * cols + vDy * rows;
        final float brZ = tlz + uDz * cols + vDz * rows;
        final float blX = tlx + vDx * rows;
        final float blY = tly + vDy * rows;
        final float blZ = tlz + vDz * rows;

        final float nx = facing.getStepX();
        final float ny = facing.getStepY();
        final float nz = facing.getStepZ();

        final int fullBright = 15728880;
        final int overlay = OverlayTexture.NO_OVERLAY;
        final int white = 0xFFFFFFFF;

        RenderType renderType = RenderTypes.entityTranslucentEmissive(state.texture);

        submitNodeCollector.submitCustomGeometry(poseStack, renderType, (pose, buffer) -> {
            buffer.addVertex(pose, tlX, tlY, tlZ).setColor(white).setUv(0f, 0f)
                    .setOverlay(overlay).setLight(fullBright).setNormal(pose, nx, ny, nz);
            buffer.addVertex(pose, blX, blY, blZ).setColor(white).setUv(0f, 1f)
                    .setOverlay(overlay).setLight(fullBright).setNormal(pose, nx, ny, nz);
            buffer.addVertex(pose, brX, brY, brZ).setColor(white).setUv(1f, 1f)
                    .setOverlay(overlay).setLight(fullBright).setNormal(pose, nx, ny, nz);
            buffer.addVertex(pose, trX, trY, trZ).setColor(white).setUv(1f, 0f)
                    .setOverlay(overlay).setLight(fullBright).setNormal(pose, nx, ny, nz);
        });
    }

    /** Drop textures for anchors that no longer exist in the client world. */
    private static void pruneStaleTextures() {
        Minecraft mc = Minecraft.getInstance();
        if (mc.level == null) return;
        Iterator<Map.Entry<BlockPos, TerminalGraphicsTexture>> it = TEXTURES.entrySet().iterator();
        while (it.hasNext()) {
            Map.Entry<BlockPos, TerminalGraphicsTexture> e = it.next();
            BlockPos p = e.getKey();
            if (!(mc.level.getBlockEntity(p) instanceof ScreenBlockEntity sbe) || !sbe.isAnchor()) {
                e.getValue().close();
                it.remove();
                LAST_DIRTY.remove(p);
            }
        }
    }
}
