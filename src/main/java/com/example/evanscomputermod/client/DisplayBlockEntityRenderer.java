package com.example.evanscomputermod.client;

import com.example.evanscomputermod.block.DisplayBlock;
import com.example.evanscomputermod.block.DisplayBlockEntity;
import com.example.evanscomputermod.config.DisplayConfig;
import com.mojang.blaze3d.vertex.PoseStack;
import com.mojang.blaze3d.vertex.VertexConsumer;
import net.minecraft.client.renderer.MultiBufferSource;
import net.minecraft.client.renderer.RenderType;
import net.minecraft.client.renderer.blockentity.BlockEntityRenderer;
import net.minecraft.client.renderer.blockentity.BlockEntityRendererProvider;
import net.minecraft.core.Direction;
import org.joml.Matrix4f;

/**
 * Renders the framebuffer texture on the front face of a Display block.
 */
public class DisplayBlockEntityRenderer implements BlockEntityRenderer<DisplayBlockEntity> {

    public DisplayBlockEntityRenderer(BlockEntityRendererProvider.Context context) {
    }

    @Override
    public void render(DisplayBlockEntity entity, float partialTick, PoseStack poseStack,
                       MultiBufferSource bufferSource, int packedLight, int packedOverlay) {
        ClientDisplayManager.DisplayClientState displayState =
                ClientDisplayManager.getDisplay(entity.getBlockPos());

        if (displayState == null) return;

        // Check render distance
        int maxDist = DisplayConfig.COMMON_SPEC.isLoaded()
                ? DisplayConfig.COMMON.displayRenderDistance.get() : 64;
        // The renderer is only called within view distance, so we trust Minecraft's culling
        // but can add our own distance check if needed.

        Direction facing = entity.getBlockState().getValue(DisplayBlock.FACING);

        poseStack.pushPose();

        // Translate and rotate based on facing direction
        // The display renders on the front face of the block, slightly offset to prevent z-fighting
        float offset = 0.005f; // Small offset to prevent z-fighting with block face

        switch (facing) {
            case NORTH -> {
                poseStack.translate(1, 0, offset);
                poseStack.mulPose(com.mojang.math.Axis.YP.rotationDegrees(180));
            }
            case SOUTH -> {
                poseStack.translate(0, 0, 1 - offset);
            }
            case WEST -> {
                poseStack.translate(offset, 0, 0);
                poseStack.mulPose(com.mojang.math.Axis.YP.rotationDegrees(90));
            }
            case EAST -> {
                poseStack.translate(1 - offset, 0, 1);
                poseStack.mulPose(com.mojang.math.Axis.YP.rotationDegrees(-90));
            }
            default -> {} // UP/DOWN not supported for horizontal display
        }

        // Draw a textured quad covering the block face
        RenderType renderType = RenderType.entityTranslucentCull(displayState.textureId);
        VertexConsumer builder = bufferSource.getBuffer(renderType);
        Matrix4f matrix = poseStack.last().pose();

        // Quad: full block face (0,0) to (1,1), texture coords (0,0) to (1,1)
        // Vertices are in counter-clockwise order when viewed from the front
        int light = 0x00F000F0; // Full brightness for the display
        int r = 255, g = 255, b = 255, a = 255;

        // Bottom-left
        builder.addVertex(matrix, 0, 0, 0)
                .setColor(r, g, b, a)
                .setUv(0, 1)
                .setOverlay(packedOverlay)
                .setLight(light)
                .setNormal(0, 0, 1);

        // Bottom-right
        builder.addVertex(matrix, 1, 0, 0)
                .setColor(r, g, b, a)
                .setUv(1, 1)
                .setOverlay(packedOverlay)
                .setLight(light)
                .setNormal(0, 0, 1);

        // Top-right
        builder.addVertex(matrix, 1, 1, 0)
                .setColor(r, g, b, a)
                .setUv(1, 0)
                .setOverlay(packedOverlay)
                .setLight(light)
                .setNormal(0, 0, 1);

        // Top-left
        builder.addVertex(matrix, 0, 1, 0)
                .setColor(r, g, b, a)
                .setUv(0, 0)
                .setOverlay(packedOverlay)
                .setLight(light)
                .setNormal(0, 0, 1);

        poseStack.popPose();
    }
}
