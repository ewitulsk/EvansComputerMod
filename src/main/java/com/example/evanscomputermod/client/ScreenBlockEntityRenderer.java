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
import net.minecraft.client.renderer.rendertype.RenderTypes;
import net.minecraft.client.renderer.state.level.CameraRenderState;
import net.minecraft.client.renderer.texture.OverlayTexture;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.resources.Identifier;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.phys.Vec3;

import java.util.HashMap;
import java.util.Iterator;
import java.util.Map;

import org.jspecify.annotations.Nullable;

/**
 * Renders every screen block's display face, plus the content texture for
 * cluster anchors that are powered on.
 *
 * <p>Per-block face composition:
 * <ol>
 *   <li>A near-black face body covering the interior of the front face.</li>
 *   <li>Cool-gray edge strips on any side where the adjacent same-facing
 *       neighbor is absent — so same-facing screens placed side-by-side
 *       merge into a continuous dark surface, even before a computer is
 *       connected.</li>
 *   <li>For cluster anchors whose {@code ScreenBlockEntity.isActive()} is
 *       true, an additional textured quad spanning the entire
 *       {@code cols×rows} rectangle, drawn with the per-cluster
 *       {@link TerminalGraphicsTexture}.</li>
 * </ol>
 *
 * <p>MC 26.1 BER API: state snapshotted in
 * {@link #extractRenderState(ScreenBlockEntity, State, float, Vec3, ModelFeatureRenderer.CrumblingOverlay)},
 * geometry submitted in
 * {@link #submit(State, PoseStack, SubmitNodeCollector, CameraRenderState)}.
 */
public class ScreenBlockEntityRenderer implements BlockEntityRenderer<ScreenBlockEntity, ScreenBlockEntityRenderer.State> {

    private static final Identifier SOLID_WHITE =
            Identifier.fromNamespaceAndPath("evanscomputermod", "block/screen_solid");

    /** Edge strip thickness in block units (1/16 = one texture pixel). */
    private static final float EDGE_T = 1.0f / 16.0f;
    /** How far to push the face body in front of the block surface to avoid z-fighting. */
    private static final float EPS_FACE = 0.001f;
    /** Edge strips sit slightly in front of the face body. */
    private static final float EPS_EDGE = 0.002f;
    /** Content quad sits in front of both face and edges so it always wins depth. */
    private static final float EPS_CONTENT = 0.003f;

    /** Near-black with a faint blue tint, same vibe as the inactive front texture's center. */
    private static final int COLOR_FACE_BODY = 0xFF06_0A16;
    /** Cool gray with slight blue tint, picked to read as an "unpowered monitor bezel". */
    private static final int COLOR_EDGE = 0xFF38_3D50;
    private static final int COLOR_WHITE = 0xFFFFFFFF;

    private static final int FULL_BRIGHT = 15728880;

    /** Per-frame state for a single screen block. */
    public static class State extends BlockEntityRenderState {
        @Nullable public Direction facing;
        public boolean connectTop, connectBottom, connectLeft, connectRight;
        public boolean anchor;
        public boolean active;
        public int cols;
        public int rows;
        @Nullable public Identifier contentTexture;
    }

    /** Shared per-anchor GPU texture cache. Keyed by anchor block position. */
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

    @Override
    public void extractRenderState(ScreenBlockEntity be, State state, float partialTicks, Vec3 cameraPosition,
                                    ModelFeatureRenderer.@Nullable CrumblingOverlay breakProgress) {
        BlockEntityRenderState.extractBase(be, state, breakProgress);

        state.facing = null;
        state.connectTop = state.connectBottom = state.connectLeft = state.connectRight = false;
        state.anchor = false;
        state.active = false;
        state.cols = 0;
        state.rows = 0;
        state.contentTexture = null;

        BlockState bs = be.getBlockState();
        if (!(bs.getBlock() instanceof ScreenBlock)) return;
        Direction facing = bs.getValue(ScreenBlock.FACING);
        state.facing = facing;
        state.anchor = be.isAnchor();
        state.active = be.isActive();
        state.cols = be.getClusterCols();
        state.rows = be.getClusterRows();

        // Face-plane connection flags, purely adjacency-based. Determines
        // which of the 4 edges of this block's face need a gray strip.
        Level level = be.getLevel();
        BlockPos pos = be.getBlockPos();
        if (level != null) {
            Direction leftDir, rightDir;
            switch (facing) {
                case NORTH -> { leftDir = Direction.EAST;  rightDir = Direction.WEST;  }
                case SOUTH -> { leftDir = Direction.WEST;  rightDir = Direction.EAST;  }
                case WEST  -> { leftDir = Direction.NORTH; rightDir = Direction.SOUTH; }
                case EAST  -> { leftDir = Direction.SOUTH; rightDir = Direction.NORTH; }
                default -> {
                    state.facing = null;
                    return;
                }
            }
            state.connectTop    = isSameFacingScreen(level, pos.above(), facing);
            state.connectBottom = isSameFacingScreen(level, pos.below(), facing);
            state.connectLeft   = isSameFacingScreen(level, pos.relative(leftDir),  facing);
            state.connectRight  = isSameFacingScreen(level, pos.relative(rightDir), facing);
        }

        // Content texture prep — only the anchor of a powered-on cluster
        // owns the GPU texture; every other screen block just draws its
        // face + edges.
        if (!state.anchor || !state.active || state.cols <= 0 || state.rows <= 0) return;

        TerminalDisplay display = be.clientDisplay;
        if (display == null) return;
        int gfxW = display.getGfxWidth();
        int gfxH = display.getGfxHeight();
        if (gfxW <= 0 || gfxH <= 0) return;
        byte[] pixels = display.getPixelData();
        int[] palette = display.getPalette();
        if (pixels == null || palette == null) return;

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
        state.contentTexture = tex.getTextureId();

        if ((System.nanoTime() & 0x7FFFFFFF) == 0) pruneStaleTextures();
    }

    private static boolean isSameFacingScreen(Level level, BlockPos pos, Direction facing) {
        BlockState s = level.getBlockState(pos);
        return s.getBlock() instanceof ScreenBlock && s.getValue(ScreenBlock.FACING) == facing;
    }

    @Override
    public void submit(State state, PoseStack poseStack, SubmitNodeCollector collector, CameraRenderState camera) {
        if (state.facing == null) return;
        Direction facing = state.facing;

        // Base face corner + axes (no eps offset — added per-quad). This
        // is the viewer-top-left corner of the block's display face in
        // local coordinates, with uD = viewer-right and vDown = (0,-1,0)
        // (screen Y increases downward; world +y is upward).
        float baseTlx, baseTly, baseTlz;
        float uDx, uDy, uDz;
        switch (facing) {
            case NORTH -> { baseTlx = 1f; baseTly = 1f; baseTlz = 0f; uDx = -1; uDy = 0; uDz =  0; }
            case SOUTH -> { baseTlx = 0f; baseTly = 1f; baseTlz = 1f; uDx =  1; uDy = 0; uDz =  0; }
            case WEST  -> { baseTlx = 0f; baseTly = 1f; baseTlz = 0f; uDx =  0; uDy = 0; uDz =  1; }
            case EAST  -> { baseTlx = 1f; baseTly = 1f; baseTlz = 1f; uDx =  0; uDy = 0; uDz = -1; }
            default -> { return; }
        }
        final float nx = facing.getStepX();
        final float ny = facing.getStepY();
        final float nz = facing.getStepZ();

        // Face body inset: shrink away from every edge that isn't
        // connected, so the body and that edge's strip don't overlap and
        // z-fight.
        final float bodyU0 = state.connectLeft   ? 0f          : EDGE_T;
        final float bodyU1 = state.connectRight  ? 1f          : 1f - EDGE_T;
        final float bodyV0 = state.connectTop    ? 0f          : EDGE_T;
        final float bodyV1 = state.connectBottom ? 1f          : 1f - EDGE_T;

        final float fBaseTlx = baseTlx, fBaseTly = baseTly, fBaseTlz = baseTlz;
        final float fUDx = uDx, fUDy = uDy, fUDz = uDz;
        final boolean cTop = state.connectTop, cBot = state.connectBottom;
        final boolean cLeft = state.connectLeft, cRight = state.connectRight;

        // Pass 1: face body + edge strips, one custom-geometry batch.
        collector.submitCustomGeometry(poseStack,
                RenderTypes.entityCutout(SOLID_WHITE),
                (pose, buffer) -> {
                    // Face body
                    emitFaceRect(pose, buffer, fBaseTlx, fBaseTly, fBaseTlz, fUDx, fUDy, fUDz,
                            bodyU0, bodyV0, bodyU1, bodyV1, EPS_FACE, nx, ny, nz, COLOR_FACE_BODY);

                    // Top strip spans the full face width
                    if (!cTop) {
                        emitFaceRect(pose, buffer, fBaseTlx, fBaseTly, fBaseTlz, fUDx, fUDy, fUDz,
                                0f, 0f, 1f, EDGE_T, EPS_EDGE, nx, ny, nz, COLOR_EDGE);
                    }
                    // Bottom strip spans the full face width
                    if (!cBot) {
                        emitFaceRect(pose, buffer, fBaseTlx, fBaseTly, fBaseTlz, fUDx, fUDy, fUDz,
                                0f, 1f - EDGE_T, 1f, 1f, EPS_EDGE, nx, ny, nz, COLOR_EDGE);
                    }
                    // Left strip clamps to the body's vertical range so
                    // corners aren't drawn twice.
                    if (!cLeft) {
                        float v0 = cTop ? 0f : EDGE_T;
                        float v1 = cBot ? 1f : 1f - EDGE_T;
                        emitFaceRect(pose, buffer, fBaseTlx, fBaseTly, fBaseTlz, fUDx, fUDy, fUDz,
                                0f, v0, EDGE_T, v1, EPS_EDGE, nx, ny, nz, COLOR_EDGE);
                    }
                    // Right strip clamps too
                    if (!cRight) {
                        float v0 = cTop ? 0f : EDGE_T;
                        float v1 = cBot ? 1f : 1f - EDGE_T;
                        emitFaceRect(pose, buffer, fBaseTlx, fBaseTly, fBaseTlz, fUDx, fUDy, fUDz,
                                1f - EDGE_T, v0, 1f, v1, EPS_EDGE, nx, ny, nz, COLOR_EDGE);
                    }
                });

        // Pass 2: content quad (anchor + active + texture loaded).
        if (state.anchor && state.active && state.contentTexture != null && state.cols > 0 && state.rows > 0) {
            final int cols = state.cols;
            final int rows = state.rows;
            collector.submitCustomGeometry(poseStack,
                    RenderTypes.entityTranslucentEmissive(state.contentTexture),
                    (pose, buffer) -> emitContentQuad(pose, buffer,
                            fBaseTlx, fBaseTly, fBaseTlz, fUDx, fUDy, fUDz,
                            cols, rows, EPS_CONTENT, nx, ny, nz));
        }
    }

    /**
     * Emit a face-local rectangle [u0,u1]×[v0,v1] as a CCW-wound quad in
     * the current pose, pushed out by {@code eps} along the face normal.
     * {@code u} maps to the viewer-right direction and {@code v} maps
     * downward (v=0 is the top of the face).
     */
    private static void emitFaceRect(PoseStack.Pose pose, VertexConsumer buf,
                                      float tlx, float tly, float tlz,
                                      float uDx, float uDy, float uDz,
                                      float u0, float v0, float u1, float v1,
                                      float eps, float nx, float ny, float nz, int color) {
        float ex = nx * eps;
        float ey = ny * eps;
        float ez = nz * eps;
        int overlay = OverlayTexture.NO_OVERLAY;

        // CCW from viewer: (u0,v0) tl → (u0,v1) bl → (u1,v1) br → (u1,v0) tr
        float x00 = tlx + uDx * u0 + ex;
        float y00 = tly - v0        + ey;
        float z00 = tlz + uDz * u0 + ez;

        float x01 = tlx + uDx * u0 + ex;
        float y01 = tly - v1        + ey;
        float z01 = tlz + uDz * u0 + ez;

        float x11 = tlx + uDx * u1 + ex;
        float y11 = tly - v1        + ey;
        float z11 = tlz + uDz * u1 + ez;

        float x10 = tlx + uDx * u1 + ex;
        float y10 = tly - v0        + ey;
        float z10 = tlz + uDz * u1 + ez;

        buf.addVertex(pose, x00, y00, z00).setColor(color).setUv(0.5f, 0.5f)
                .setOverlay(overlay).setLight(FULL_BRIGHT).setNormal(pose, nx, ny, nz);
        buf.addVertex(pose, x01, y01, z01).setColor(color).setUv(0.5f, 0.5f)
                .setOverlay(overlay).setLight(FULL_BRIGHT).setNormal(pose, nx, ny, nz);
        buf.addVertex(pose, x11, y11, z11).setColor(color).setUv(0.5f, 0.5f)
                .setOverlay(overlay).setLight(FULL_BRIGHT).setNormal(pose, nx, ny, nz);
        buf.addVertex(pose, x10, y10, z10).setColor(color).setUv(0.5f, 0.5f)
                .setOverlay(overlay).setLight(FULL_BRIGHT).setNormal(pose, nx, ny, nz);
    }

    /**
     * Emit the anchor's content quad, which spans the entire cluster in
     * world space (not just the anchor block). Standard UVs:
     * (0,0)=tl → (1,1)=br; textured with the cluster's dynamic GPU texture.
     */
    private static void emitContentQuad(PoseStack.Pose pose, VertexConsumer buf,
                                         float tlx, float tly, float tlz,
                                         float uDx, float uDy, float uDz,
                                         int cols, int rows, float eps,
                                         float nx, float ny, float nz) {
        float ex = nx * eps;
        float ey = ny * eps;
        float ez = nz * eps;
        int overlay = OverlayTexture.NO_OVERLAY;
        float w = cols;
        float h = rows;

        float tlX = tlx + ex,          tlY = tly + ey,        tlZ = tlz + ez;
        float blX = tlx + ex,          blY = tly - h + ey,    blZ = tlz + ez;
        float brX = tlx + uDx * w + ex, brY = tly - h + ey,   brZ = tlz + uDz * w + ez;
        float trX = tlx + uDx * w + ex, trY = tly + ey,       trZ = tlz + uDz * w + ez;

        buf.addVertex(pose, tlX, tlY, tlZ).setColor(COLOR_WHITE).setUv(0f, 0f)
                .setOverlay(overlay).setLight(FULL_BRIGHT).setNormal(pose, nx, ny, nz);
        buf.addVertex(pose, blX, blY, blZ).setColor(COLOR_WHITE).setUv(0f, 1f)
                .setOverlay(overlay).setLight(FULL_BRIGHT).setNormal(pose, nx, ny, nz);
        buf.addVertex(pose, brX, brY, brZ).setColor(COLOR_WHITE).setUv(1f, 1f)
                .setOverlay(overlay).setLight(FULL_BRIGHT).setNormal(pose, nx, ny, nz);
        buf.addVertex(pose, trX, trY, trZ).setColor(COLOR_WHITE).setUv(1f, 0f)
                .setOverlay(overlay).setLight(FULL_BRIGHT).setNormal(pose, nx, ny, nz);
    }

    /** Drop cached textures for anchors that no longer exist in the client world. */
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
