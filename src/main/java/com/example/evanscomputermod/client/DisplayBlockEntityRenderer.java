package com.example.evanscomputermod.client;

import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.block.DisplayBlock;
import com.example.evanscomputermod.block.DisplayBlockEntity;
import com.mojang.blaze3d.platform.NativeImage;
import com.mojang.blaze3d.vertex.PoseStack;
import com.mojang.blaze3d.vertex.VertexConsumer;
import net.minecraft.client.Minecraft;
import net.minecraft.client.renderer.MultiBufferSource;
import net.minecraft.client.renderer.RenderType;
import net.minecraft.client.renderer.blockentity.BlockEntityRenderer;
import net.minecraft.client.renderer.blockentity.BlockEntityRendererProvider;
import net.minecraft.client.renderer.texture.DynamicTexture;
import net.minecraft.client.renderer.texture.OverlayTexture;
import net.minecraft.core.Direction;
import net.minecraft.resources.ResourceLocation;

/**
 * Renders the framebuffer texture on the front face and a solid casing on all other faces.
 * Uses ENTITYBLOCK_ANIMATED so this BER handles ALL rendering for the display block.
 */
public class DisplayBlockEntityRenderer implements BlockEntityRenderer<DisplayBlockEntity> {

    /** 1x1 dark gray texture for the casing faces. */
    private static ResourceLocation casingTextureId;
    /** 1x1 black texture for the "off" screen. */
    private static ResourceLocation screenOffTextureId;

    public DisplayBlockEntityRenderer(BlockEntityRendererProvider.Context context) {
        initTextures();
    }

    private static void initTextures() {
        if (casingTextureId != null) return;

        // Create a 1x1 dark gray pixel for casing
        NativeImage casingImg = new NativeImage(NativeImage.Format.RGBA, 1, 1, false);
        casingImg.setPixelRGBA(0, 0, 0xFF282828); // ABGR: opaque dark gray (40,40,40)
        DynamicTexture casingTex = new DynamicTexture(casingImg);
        casingTextureId = Minecraft.getInstance().getTextureManager()
                .register("evanscomputermod_casing", casingTex);

        // Create a 1x1 black pixel for screen-off
        NativeImage screenImg = new NativeImage(NativeImage.Format.RGBA, 1, 1, false);
        screenImg.setPixelRGBA(0, 0, 0xFF000000); // ABGR: opaque black
        DynamicTexture screenTex = new DynamicTexture(screenImg);
        screenOffTextureId = Minecraft.getInstance().getTextureManager()
                .register("evanscomputermod_screen_off", screenTex);
    }

    @Override
    public void render(DisplayBlockEntity entity, float partialTick, PoseStack poseStack,
                       MultiBufferSource bufferSource, int packedLight, int packedOverlay) {
        Direction facing = entity.getBlockState().getValue(DisplayBlock.FACING);
        ClientDisplayManager.DisplayClientState displayState =
                ClientDisplayManager.getDisplay(entity.getBlockPos());

        int light = 0x00F000F0; // Full brightness for display
        int overlay = OverlayTexture.NO_OVERLAY;

        // --- Render casing (5 non-display faces) ---
        initTextures();
        RenderType casingType = RenderType.entitySolid(casingTextureId);
        VertexConsumer casing = bufferSource.getBuffer(casingType);
        PoseStack.Pose pose = poseStack.last();

        // Render all 6 faces of the unit cube, skipping the display face
        for (Direction dir : Direction.values()) {
            if (dir == facing) continue; // Display face rendered separately
            renderFace(casing, pose, dir, packedLight, overlay, 40, 40, 40);
        }

        // --- Render display face ---
        ResourceLocation screenTexture = (displayState != null) ? displayState.textureId : screenOffTextureId;
        int screenLight = 0x00F000F0; // Full brightness for the screen
        RenderType screenType = RenderType.entitySolid(screenTexture);
        VertexConsumer screen = bufferSource.getBuffer(screenType);

        renderFace(screen, pose, facing, screenLight, overlay, 255, 255, 255);
    }

    /**
     * Renders a single face of the unit cube.
     */
    private void renderFace(VertexConsumer builder, PoseStack.Pose pose, Direction dir,
                            int light, int overlay, int r, int g, int b) {
        // Each face is defined by 4 vertices with appropriate normals
        float nx = dir.getStepX();
        float ny = dir.getStepY();
        float nz = dir.getStepZ();

        switch (dir) {
            case SOUTH -> { // +Z face
                quad(builder, pose, light, overlay, r, g, b, nx, ny, nz,
                        0, 0, 1,   1, 0, 1,   1, 1, 1,   0, 1, 1);
            }
            case NORTH -> { // -Z face
                quad(builder, pose, light, overlay, r, g, b, nx, ny, nz,
                        1, 0, 0,   0, 0, 0,   0, 1, 0,   1, 1, 0);
            }
            case EAST -> { // +X face
                quad(builder, pose, light, overlay, r, g, b, nx, ny, nz,
                        1, 0, 1,   1, 0, 0,   1, 1, 0,   1, 1, 1);
            }
            case WEST -> { // -X face
                quad(builder, pose, light, overlay, r, g, b, nx, ny, nz,
                        0, 0, 0,   0, 0, 1,   0, 1, 1,   0, 1, 0);
            }
            case UP -> { // +Y face
                quad(builder, pose, light, overlay, r, g, b, nx, ny, nz,
                        0, 1, 0,   0, 1, 1,   1, 1, 1,   1, 1, 0);
            }
            case DOWN -> { // -Y face
                quad(builder, pose, light, overlay, r, g, b, nx, ny, nz,
                        0, 0, 1,   0, 0, 0,   1, 0, 0,   1, 0, 1);
            }
        }
    }

    /**
     * Emits 4 vertices for a quad with the given positions, UVs mapped to full texture.
     */
    private void quad(VertexConsumer builder, PoseStack.Pose pose,
                      int light, int overlay, int r, int g, int b,
                      float nx, float ny, float nz,
                      float x0, float y0, float z0,
                      float x1, float y1, float z1,
                      float x2, float y2, float z2,
                      float x3, float y3, float z3) {
        builder.addVertex(pose, x0, y0, z0).setColor(r, g, b, 255).setUv(0, 1)
                .setOverlay(overlay).setLight(light).setNormal(pose, nx, ny, nz);
        builder.addVertex(pose, x1, y1, z1).setColor(r, g, b, 255).setUv(1, 1)
                .setOverlay(overlay).setLight(light).setNormal(pose, nx, ny, nz);
        builder.addVertex(pose, x2, y2, z2).setColor(r, g, b, 255).setUv(1, 0)
                .setOverlay(overlay).setLight(light).setNormal(pose, nx, ny, nz);
        builder.addVertex(pose, x3, y3, z3).setColor(r, g, b, 255).setUv(0, 0)
                .setOverlay(overlay).setLight(light).setNormal(pose, nx, ny, nz);
    }
}
