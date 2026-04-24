package com.example.evanscomputermod.client;

import com.example.evanscomputermod.block.MissileLauncherBlockEntity;
import com.mojang.blaze3d.vertex.PoseStack;
import com.mojang.blaze3d.vertex.VertexConsumer;
import com.mojang.math.Axis;
//? if >=26.1 {
import net.minecraft.client.renderer.SubmitNodeCollector;
import net.minecraft.client.renderer.blockentity.state.BlockEntityRenderState;
import net.minecraft.client.renderer.feature.ModelFeatureRenderer;
import net.minecraft.client.renderer.rendertype.RenderTypes;
import net.minecraft.client.renderer.state.level.CameraRenderState;
//?} else {
/*import net.minecraft.client.renderer.MultiBufferSource;
import net.minecraft.client.renderer.RenderType;*///?}
import net.minecraft.client.renderer.blockentity.BlockEntityRenderer;
import net.minecraft.client.renderer.blockentity.BlockEntityRendererProvider;
import net.minecraft.client.renderer.texture.OverlayTexture;
import net.minecraft.resources.Identifier;
import net.minecraft.world.phys.Vec3;

import org.jetbrains.annotations.Nullable;

/**
 * Renders the missile launcher block with an animated barrel that tracks the
 * bearing and elevation stored in the block entity.
 */
//? if >=26.1 {
public class MissileLauncherBlockEntityRenderer implements BlockEntityRenderer<MissileLauncherBlockEntity, MissileLauncherBlockEntityRenderer.State> {
//?} else
/*public class MissileLauncherBlockEntityRenderer implements BlockEntityRenderer<MissileLauncherBlockEntity> {*/

    //? if >=26.1 {
    public static class State extends BlockEntityRenderState {
        public float bearing   = 0f;
        public float elevation = 45f;
        public boolean loaded  = false;
    }
    //?}

    private static final Identifier TEXTURE =
            Identifier.fromNamespaceAndPath("evanscomputermod", "textures/block/launcher_side.png");

    public MissileLauncherBlockEntityRenderer(BlockEntityRendererProvider.Context context) {}

    //? if >=26.1 {
    @Override
    public State createRenderState() { return new State(); }

    @Override
    public void extractRenderState(MissileLauncherBlockEntity be, State state, float partialTick,
                                   Vec3 cameraPosition,
                                   ModelFeatureRenderer.@Nullable CrumblingOverlay breakProgress) {
        BlockEntityRenderState.extractBase(be, state, breakProgress);
        state.bearing   = be.getBearing();
        state.elevation = be.getElevation();
        state.loaded    = be.isLoaded();
    }

    @Override
    public void submit(State state, PoseStack ps, SubmitNodeCollector collector, CameraRenderState camera) {
        ps.pushPose();
        ps.translate(0.5, 0.0, 0.5);
        collector.submitCustomGeometry(ps, RenderTypes.entitySolid(TEXTURE), (pose, buf) ->
                renderBox(pose, buf, state.lightCoords, OverlayTexture.NO_OVERLAY, 0xFF303030,
                        -0.5f, 0.0f, -0.5f, 0.5f, 0.35f, 0.5f));
        ps.popPose();

        ps.pushPose();
        ps.translate(0.5, 0.35, 0.5);
        ps.mulPose(Axis.YP.rotationDegrees(-state.bearing));
        ps.mulPose(Axis.XP.rotationDegrees(-state.elevation));
        int barrelColor = state.loaded ? 0xFF22BB44 : 0xFF607060;
        collector.submitCustomGeometry(ps, RenderTypes.entitySolid(TEXTURE), (pose, buf) ->
                renderBox(pose, buf, state.lightCoords, OverlayTexture.NO_OVERLAY, barrelColor,
                        -0.08f, 0.0f, -0.08f, 0.08f, 0.65f, 0.08f));
        ps.popPose();
    }
    //?} else {
    /*@Override
    public void render(MissileLauncherBlockEntity be, float partialTick,
                       PoseStack ps, MultiBufferSource buf, int light, int overlay) {
        float bearing   = be.getBearing();
        float elevation = be.getElevation();
        VertexConsumer vc = buf.getBuffer(RenderType.entitySolid(TEXTURE));
        ps.pushPose();
        ps.translate(0.5, 0.0, 0.5);
        renderBox(ps.last(), vc, light, overlay, 0xFF303030, -0.5f, 0.0f, -0.5f, 0.5f, 0.35f, 0.5f);
        ps.popPose();
        ps.pushPose();
        ps.translate(0.5, 0.35, 0.5);
        ps.mulPose(Axis.YP.rotationDegrees(-bearing));
        ps.mulPose(Axis.XP.rotationDegrees(-elevation));
        int barrelColor = be.isLoaded() ? 0xFF22BB44 : 0xFF607060;
        renderBox(ps.last(), vc, light, overlay, barrelColor, -0.08f, 0.0f, -0.08f, 0.08f, 0.65f, 0.08f);
        ps.popPose();
    }*/
    //?}

    private void renderBox(PoseStack.Pose pose, VertexConsumer vc, int light, int overlay, int argb,
                           float x0, float y0, float z0, float x1, float y1, float z1) {
        int r = (argb >> 16) & 0xFF, g = (argb >> 8) & 0xFF, b = argb & 0xFF, a = (argb >> 24) & 0xFF;
        quad(pose, vc, light, overlay, r, g, b, a, x0, y0, z0, x1, y0, z0, x1, y0, z1, x0, y0, z1,  0, -1,  0);
        quad(pose, vc, light, overlay, r, g, b, a, x0, y1, z1, x1, y1, z1, x1, y1, z0, x0, y1, z0,  0,  1,  0);
        quad(pose, vc, light, overlay, r, g, b, a, x1, y1, z0, x1, y0, z0, x0, y0, z0, x0, y1, z0,  0,  0, -1);
        quad(pose, vc, light, overlay, r, g, b, a, x0, y1, z1, x0, y0, z1, x1, y0, z1, x1, y1, z1,  0,  0,  1);
        quad(pose, vc, light, overlay, r, g, b, a, x0, y1, z0, x0, y0, z0, x0, y0, z1, x0, y1, z1, -1,  0,  0);
        quad(pose, vc, light, overlay, r, g, b, a, x1, y1, z1, x1, y0, z1, x1, y0, z0, x1, y1, z0,  1,  0,  0);
    }

    private void quad(PoseStack.Pose pose, VertexConsumer vc, int light, int overlay,
                      int r, int g, int b, int a,
                      float x0, float y0, float z0, float x1, float y1, float z1,
                      float x2, float y2, float z2, float x3, float y3, float z3,
                      float nx, float ny, float nz) {
        vtx(pose, vc, light, overlay, x0, y0, z0, nx, ny, nz, 0, 1, r, g, b, a);
        vtx(pose, vc, light, overlay, x1, y1, z1, nx, ny, nz, 0, 0, r, g, b, a);
        vtx(pose, vc, light, overlay, x2, y2, z2, nx, ny, nz, 1, 0, r, g, b, a);
        vtx(pose, vc, light, overlay, x3, y3, z3, nx, ny, nz, 1, 1, r, g, b, a);
    }

    private void vtx(PoseStack.Pose pose, VertexConsumer vc, int light, int overlay,
                     float x, float y, float z, float nx, float ny, float nz,
                     float u, float v, int r, int g, int b, int a) {
        vc.addVertex(pose, x, y, z).setColor(r, g, b, a).setUv(u, v)
          .setOverlay(overlay).setLight(light).setNormal(pose, nx, ny, nz);
    }
}
