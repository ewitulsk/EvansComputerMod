package com.example.evanscomputermod.client;

import com.example.evanscomputermod.block.ScreenBlock;
import com.example.evanscomputermod.block.ScreenBlockEntity;
import com.example.evanscomputermod.computer.TerminalDisplay;
import com.mojang.blaze3d.vertex.PoseStack;
import com.mojang.blaze3d.vertex.VertexConsumer;
import net.minecraft.client.Minecraft;
//? if >=26.1 {
import net.minecraft.client.renderer.SubmitNodeCollector;
//?}
import net.minecraft.client.renderer.blockentity.BlockEntityRenderer;
import net.minecraft.client.renderer.blockentity.BlockEntityRendererProvider;
//? if >=26.1 {
import net.minecraft.client.renderer.blockentity.state.BlockEntityRenderState;
import net.minecraft.client.renderer.feature.ModelFeatureRenderer;
import net.minecraft.client.renderer.rendertype.RenderType;
import net.minecraft.client.renderer.rendertype.RenderTypes;
import net.minecraft.client.renderer.state.level.CameraRenderState;
//?} else {
/*import net.minecraft.client.renderer.MultiBufferSource;
import net.minecraft.client.renderer.RenderType;
*///?}
import net.minecraft.client.renderer.texture.OverlayTexture;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.resources.Identifier;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.phys.AABB;
import net.minecraft.world.phys.Vec3;

import java.util.HashMap;
import java.util.Iterator;
import java.util.Map;

import org.jetbrains.annotations.Nullable;

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
 * <p>MC 26.1 BER API: state snapshotted in extractRenderState,
 * geometry submitted in submit(). MC 1.21.1: single render() method.
 */
//? if >=26.1 {
public class ScreenBlockEntityRenderer implements BlockEntityRenderer<ScreenBlockEntity, ScreenBlockEntityRenderer.State> {
//?} else
/*public class ScreenBlockEntityRenderer implements BlockEntityRenderer<ScreenBlockEntity> {*/

    private static final Identifier SOLID_WHITE =
            Identifier.fromNamespaceAndPath("evanscomputermod", "block/screen_solid");

    /**
     * RenderType for the per-block face body + edge strips. Cached as a
     * static final to avoid per-frame construction churn.
     */
    //? if >=26.1 {
    private static final RenderType FACE_RENDER_TYPE = RenderTypes.entityCutout(SOLID_WHITE);
    //?} else
    /*private static final RenderType FACE_RENDER_TYPE = RenderType.entityCutout(SOLID_WHITE);*/

    /**
     * Per-content-texture RenderType cache.
     */
    private static final Map<Identifier, RenderType> CONTENT_RENDER_TYPES = new HashMap<>();

    private static int pruneTick = 0;

    private static RenderType contentRenderType(Identifier tex) {
        RenderType rt = CONTENT_RENDER_TYPES.get(tex);
        if (rt == null) {
            //? if >=26.1 {
            rt = RenderTypes.entityTranslucentEmissive(tex);
            //?} else
            /*rt = RenderType.entityTranslucentEmissive(tex);*/
            CONTENT_RENDER_TYPES.put(tex, rt);
        }
        return rt;
    }

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

    //? if >=26.1 {
    /** Per-frame state for a single screen block (26.1 only). */
    public static class State extends BlockEntityRenderState {
        @Nullable public Direction facing;
        public boolean connectTop, connectBottom, connectLeft, connectRight;
        public boolean anchor;
        public boolean active;
        public int cols;
        public int rows;
        @Nullable public Identifier contentTexture;
    }
    //?} else {
    /*public static class State {
        @Nullable public Direction facing;
        public boolean connectTop, connectBottom, connectLeft, connectRight;
        public boolean anchor;
        public boolean active;
        public int cols;
        public int rows;
        @Nullable public Identifier contentTexture;
    }*/
    //?}

    /** Shared per-anchor GPU texture cache. Keyed by anchor block position. */
    private static final Map<BlockPos, TerminalGraphicsTexture> TEXTURES = new HashMap<>();
    /** Last dirty counters uploaded, per anchor. */
    private static final Map<BlockPos, long[]> LAST_DIRTY = new HashMap<>();

    public ScreenBlockEntityRenderer(BlockEntityRendererProvider.Context context) {
    }

    //? if >=26.1 {
    @Override
    public State createRenderState() {
        return new State();
    }
    //?}

    @Override
    public int getViewDistance() {
        return 128;
    }

    /**
     * Override the default single-block AABB so that frustum culling knows
     * the anchor's content quad extends across the whole cluster.
     */
    @Override
    public AABB getRenderBoundingBox(ScreenBlockEntity be) {
        BlockPos p = be.getBlockPos();
        AABB unit = new AABB(p.getX(), p.getY(), p.getZ(),
                p.getX() + 1, p.getY() + 1, p.getZ() + 1);
        if (!be.isAnchor()) {
            return unit;
        }
        int cols = be.getClusterCols();
        int rows = be.getClusterRows();
        if (cols <= 0 || rows <= 0) return unit;

        BlockState bs = be.getBlockState();
        if (!(bs.getBlock() instanceof ScreenBlock)) return unit;
        Direction facing = bs.getValue(ScreenBlock.FACING);

        double minX = p.getX();
        double minY = p.getY();
        double minZ = p.getZ();
        double maxX = p.getX() + 1;
        double maxY = p.getY() + 1;
        double maxZ = p.getZ() + 1;

        int du = cols - 1;
        int dv = rows - 1;

        switch (facing) {
            case NORTH -> { minX -= du; minY -= dv; }
            case SOUTH -> { maxX += du; minY -= dv; }
            case WEST  -> { maxZ += du; minY -= dv; }
            case EAST  -> { minZ -= du; minY -= dv; }
            default -> {}
        }
        return new AABB(minX, minY, minZ, maxX, maxY, maxZ);
    }

    /**
     * Populate the per-frame state from the live block entity. Shared
     * between the 26.1 extractRenderState path and the 1.21.1 render path.
     */
    private void fillState(ScreenBlockEntity be, State state) {
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

        if (!state.anchor || !state.active || state.cols <= 0 || state.rows <= 0) return;

        TerminalDisplay display = be.clientDisplay;
        if (display == null) return;
        int gfxW = display.getGfxWidth();
        int gfxH = display.getGfxHeight();
        if (gfxW <= 0 || gfxH <= 0) return;
        byte[] pixels = display.getPixelData();
        int[] palette = display.getPalette();
        int pixelFormat = display.getPixelFormat();
        if (pixels == null) return;
        if (pixelFormat == TerminalDisplay.PIXEL_FORMAT_INDEXED8 && palette == null) return;

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
            tex.updateFull(pixelFormat, pixels, palette);
            lastDirty[0] = pixelDirty;
            lastDirty[1] = paletteDirty;
        }
        state.contentTexture = tex.getTextureId();

        if (++pruneTick >= 600) {
            pruneTick = 0;
            pruneStaleTextures();
        }
    }

    //? if >=26.1 {
    @Override
    public void extractRenderState(ScreenBlockEntity be, State state, float partialTicks, Vec3 cameraPosition,
                                    ModelFeatureRenderer.@Nullable CrumblingOverlay breakProgress) {
        BlockEntityRenderState.extractBase(be, state, breakProgress);
        fillState(be, state);
    }
    //?}

    private static boolean isSameFacingScreen(Level level, BlockPos pos, Direction facing) {
        BlockState s = level.getBlockState(pos);
        return s.getBlock() instanceof ScreenBlock && s.getValue(ScreenBlock.FACING) == facing;
    }

    //? if >=26.1 {
    @Override
    public void submit(State state, PoseStack poseStack, SubmitNodeCollector collector, CameraRenderState camera) {
        if (state.facing == null) return;
        Direction facing = state.facing;

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

        final float bodyU0 = state.connectLeft   ? 0f          : EDGE_T;
        final float bodyU1 = state.connectRight  ? 1f          : 1f - EDGE_T;
        final float bodyV0 = state.connectTop    ? 0f          : EDGE_T;
        final float bodyV1 = state.connectBottom ? 1f          : 1f - EDGE_T;

        final float fBaseTlx = baseTlx, fBaseTly = baseTly, fBaseTlz = baseTlz;
        final float fUDx = uDx, fUDy = uDy, fUDz = uDz;
        final boolean cTop = state.connectTop, cBot = state.connectBottom;
        final boolean cLeft = state.connectLeft, cRight = state.connectRight;

        collector.submitCustomGeometry(poseStack,
                FACE_RENDER_TYPE,
                (pose, buffer) -> {
                    emitFaceRect(pose, buffer, fBaseTlx, fBaseTly, fBaseTlz, fUDx, fUDy, fUDz,
                            bodyU0, bodyV0, bodyU1, bodyV1, EPS_FACE, nx, ny, nz, COLOR_FACE_BODY);

                    if (!cTop) {
                        emitFaceRect(pose, buffer, fBaseTlx, fBaseTly, fBaseTlz, fUDx, fUDy, fUDz,
                                0f, 0f, 1f, EDGE_T, EPS_EDGE, nx, ny, nz, COLOR_EDGE);
                    }
                    if (!cBot) {
                        emitFaceRect(pose, buffer, fBaseTlx, fBaseTly, fBaseTlz, fUDx, fUDy, fUDz,
                                0f, 1f - EDGE_T, 1f, 1f, EPS_EDGE, nx, ny, nz, COLOR_EDGE);
                    }
                    if (!cLeft) {
                        float v0 = cTop ? 0f : EDGE_T;
                        float v1 = cBot ? 1f : 1f - EDGE_T;
                        emitFaceRect(pose, buffer, fBaseTlx, fBaseTly, fBaseTlz, fUDx, fUDy, fUDz,
                                0f, v0, EDGE_T, v1, EPS_EDGE, nx, ny, nz, COLOR_EDGE);
                    }
                    if (!cRight) {
                        float v0 = cTop ? 0f : EDGE_T;
                        float v1 = cBot ? 1f : 1f - EDGE_T;
                        emitFaceRect(pose, buffer, fBaseTlx, fBaseTly, fBaseTlz, fUDx, fUDy, fUDz,
                                1f - EDGE_T, v0, 1f, v1, EPS_EDGE, nx, ny, nz, COLOR_EDGE);
                    }
                });

        if (state.anchor && state.active && state.contentTexture != null && state.cols > 0 && state.rows > 0) {
            final int cols = state.cols;
            final int rows = state.rows;
            collector.submitCustomGeometry(poseStack,
                    contentRenderType(state.contentTexture),
                    (pose, buffer) -> emitContentQuad(pose, buffer,
                            fBaseTlx, fBaseTly, fBaseTlz, fUDx, fUDy, fUDz,
                            cols, rows, EPS_CONTENT, nx, ny, nz));
        }
    }
    //?} else {
    /*@Override
    public void render(ScreenBlockEntity be, float partialTick, PoseStack poseStack,
                       net.minecraft.client.renderer.MultiBufferSource bufferSource,
                       int packedLight, int packedOverlay) {
        State state = new State();
        fillState(be, state);
        if (state.facing == null) return;
        Direction facing = state.facing;

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

        final float bodyU0 = state.connectLeft   ? 0f          : EDGE_T;
        final float bodyU1 = state.connectRight  ? 1f          : 1f - EDGE_T;
        final float bodyV0 = state.connectTop    ? 0f          : EDGE_T;
        final float bodyV1 = state.connectBottom ? 1f          : 1f - EDGE_T;

        PoseStack.Pose pose = poseStack.last();
        VertexConsumer faceBuf = bufferSource.getBuffer(FACE_RENDER_TYPE);
        emitFaceRect(pose, faceBuf, baseTlx, baseTly, baseTlz, uDx, uDy, uDz,
                bodyU0, bodyV0, bodyU1, bodyV1, EPS_FACE, nx, ny, nz, COLOR_FACE_BODY);
        if (!state.connectTop) {
            emitFaceRect(pose, faceBuf, baseTlx, baseTly, baseTlz, uDx, uDy, uDz,
                    0f, 0f, 1f, EDGE_T, EPS_EDGE, nx, ny, nz, COLOR_EDGE);
        }
        if (!state.connectBottom) {
            emitFaceRect(pose, faceBuf, baseTlx, baseTly, baseTlz, uDx, uDy, uDz,
                    0f, 1f - EDGE_T, 1f, 1f, EPS_EDGE, nx, ny, nz, COLOR_EDGE);
        }
        if (!state.connectLeft) {
            float v0 = state.connectTop ? 0f : EDGE_T;
            float v1 = state.connectBottom ? 1f : 1f - EDGE_T;
            emitFaceRect(pose, faceBuf, baseTlx, baseTly, baseTlz, uDx, uDy, uDz,
                    0f, v0, EDGE_T, v1, EPS_EDGE, nx, ny, nz, COLOR_EDGE);
        }
        if (!state.connectRight) {
            float v0 = state.connectTop ? 0f : EDGE_T;
            float v1 = state.connectBottom ? 1f : 1f - EDGE_T;
            emitFaceRect(pose, faceBuf, baseTlx, baseTly, baseTlz, uDx, uDy, uDz,
                    1f - EDGE_T, v0, 1f, v1, EPS_EDGE, nx, ny, nz, COLOR_EDGE);
        }

        if (state.anchor && state.active && state.contentTexture != null && state.cols > 0 && state.rows > 0) {
            VertexConsumer contentBuf = bufferSource.getBuffer(contentRenderType(state.contentTexture));
            emitContentQuad(pose, contentBuf, baseTlx, baseTly, baseTlz, uDx, uDy, uDz,
                    state.cols, state.rows, EPS_CONTENT, nx, ny, nz);
        }
    }*///?}

    /**
     * Emit a face-local rectangle [u0,u1]×[v0,v1] as a CCW-wound quad in
     * the current pose, pushed out by {@code eps} along the face normal.
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
     * world space (not just the anchor block).
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
                Identifier texId = e.getValue().getTextureId();
                e.getValue().close();
                it.remove();
                LAST_DIRTY.remove(p);
                if (texId != null) {
                    CONTENT_RENDER_TYPES.remove(texId);
                }
            }
        }
    }
}
