package com.example.evanscomputermod.client;

import com.example.evanscomputermod.entity.MissileEntity;
import com.mojang.blaze3d.vertex.PoseStack;
import com.mojang.blaze3d.vertex.VertexConsumer;
import com.mojang.math.Axis;
//? if >=26.1 {
import net.minecraft.client.renderer.SubmitNodeCollector;
import net.minecraft.client.renderer.entity.state.EntityRenderState;
import net.minecraft.client.renderer.rendertype.RenderTypes;
import net.minecraft.client.renderer.state.level.CameraRenderState;
//?} else {
/*import net.minecraft.client.renderer.MultiBufferSource;
import net.minecraft.client.renderer.RenderType;*///?}
import net.minecraft.client.renderer.entity.EntityRenderer;
import net.minecraft.client.renderer.entity.EntityRendererProvider;
import net.minecraft.client.renderer.texture.OverlayTexture;
import net.minecraft.resources.Identifier;
import net.minecraft.util.Mth;

/**
 * Renders the missile entity as a simple elongated box oriented along its
 * velocity vector. Flame particles are already broadcast server-side.
 */
//? if >=26.1 {
public class MissileEntityRenderer extends EntityRenderer<MissileEntity, MissileEntityRenderer.State> {
//?} else
/*public class MissileEntityRenderer extends EntityRenderer<MissileEntity> {*/

    //? if >=26.1 {
    public static class State extends EntityRenderState {
        public float interpYaw   = 0f;
        public float interpPitch = 0f;
    }
    //?}

    private static final Identifier TEXTURE =
            Identifier.fromNamespaceAndPath("evanscomputermod", "textures/entity/missile.png");

    public MissileEntityRenderer(EntityRendererProvider.Context context) {
        super(context);
    }

    //? if >=26.1 {
    @Override
    protected boolean affectedByCulling(MissileEntity entity) { return false; }

    @Override
    public State createRenderState() { return new State(); }

    @Override
    public void extractRenderState(MissileEntity entity, State state, float partialTick) {
        super.extractRenderState(entity, state, partialTick);
        state.interpYaw   = Mth.lerp(partialTick, entity.yRotO, entity.getYRot());
        state.interpPitch = Mth.lerp(partialTick, entity.xRotO, entity.getXRot());
    }

    @Override
    public void submit(State state, PoseStack poseStack, SubmitNodeCollector collector, CameraRenderState camera) {
        poseStack.pushPose();
        poseStack.mulPose(Axis.YP.rotationDegrees(-state.interpYaw));
        poseStack.mulPose(Axis.XP.rotationDegrees(state.interpPitch));
        collector.submitCustomGeometry(poseStack, RenderTypes.entitySolid(TEXTURE), (pose, buf) ->
                renderBox(pose, buf, state.lightCoords, OverlayTexture.NO_OVERLAY,
                        -0.075f, -0.5f, -0.075f, 0.075f, 0.5f, 0.075f));
        poseStack.popPose();
    }
    //?} else {
    /*@Override
    public Identifier getTextureLocation(MissileEntity entity) {
        return TEXTURE;
    }

    @Override
    public void render(MissileEntity entity, float yaw, float partialTick,
                       PoseStack poseStack, MultiBufferSource bufferSource, int light) {
        super.render(entity, yaw, partialTick, poseStack, bufferSource, light);
        poseStack.pushPose();
        float interpYaw   = Mth.lerp(partialTick, entity.yRotO, entity.getYRot());
        float interpPitch = Mth.lerp(partialTick, entity.xRotO, entity.getXRot());
        poseStack.mulPose(Axis.YP.rotationDegrees(-interpYaw));
        poseStack.mulPose(Axis.XP.rotationDegrees(interpPitch));
        VertexConsumer vc = bufferSource.getBuffer(RenderType.entitySolid(TEXTURE));
        renderBox(poseStack.last(), vc, light, OverlayTexture.NO_OVERLAY,
                -0.075f, -0.5f, -0.075f, 0.075f, 0.5f, 0.075f);
        poseStack.popPose();
    }*/
    //?}

    private void renderBox(PoseStack.Pose pose, VertexConsumer vc, int light, int overlay,
                           float x0, float y0, float z0, float x1, float y1, float z1) {
        int body = 0xFF404040;
        int nose = 0xFFCC2222;
        quad(pose, vc, light, overlay, body, x0, y0, z0, x1, y0, z0, x1, y0, z1, x0, y0, z1,  0, -1,  0);
        quad(pose, vc, light, overlay, nose, x0, y1, z1, x1, y1, z1, x1, y1, z0, x0, y1, z0,  0,  1,  0);
        quad(pose, vc, light, overlay, body, x1, y1, z0, x1, y0, z0, x0, y0, z0, x0, y1, z0,  0,  0, -1);
        quad(pose, vc, light, overlay, body, x0, y1, z1, x0, y0, z1, x1, y0, z1, x1, y1, z1,  0,  0,  1);
        quad(pose, vc, light, overlay, body, x0, y1, z0, x0, y0, z0, x0, y0, z1, x0, y1, z1, -1,  0,  0);
        quad(pose, vc, light, overlay, body, x1, y1, z1, x1, y0, z1, x1, y0, z0, x1, y1, z0,  1,  0,  0);
    }

    private void quad(PoseStack.Pose pose, VertexConsumer vc, int light, int overlay, int argb,
                      float x0, float y0, float z0, float x1, float y1, float z1,
                      float x2, float y2, float z2, float x3, float y3, float z3,
                      float nx, float ny, float nz) {
        int r = (argb >> 16) & 0xFF, g = (argb >> 8) & 0xFF, b = argb & 0xFF, a = (argb >> 24) & 0xFF;
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
