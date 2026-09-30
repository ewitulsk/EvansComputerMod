package com.example.evanscomputermod.sensor.client;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.sensor.LidarSensorBlock;
import com.example.evanscomputermod.sensor.LidarSensorBlockEntity;
import com.mojang.blaze3d.vertex.PoseStack;
import com.mojang.math.Axis;
import net.minecraft.client.Minecraft;
import net.minecraft.client.renderer.MultiBufferSource;
import net.minecraft.client.renderer.RenderType;
import net.minecraft.client.renderer.blockentity.BlockEntityRenderer;
import net.minecraft.client.renderer.blockentity.BlockEntityRendererProvider;
import net.minecraft.client.resources.model.ModelResourceLocation;
import net.minecraft.world.phys.Vec3;

/**
 * Draws a lidar's head (the static model is only the mount). The head faces
 * the sensor's forward and spins about the up axis while it is scanning.
 */
public class LidarSensorRenderer implements BlockEntityRenderer<LidarSensorBlockEntity> {
    public static final ModelResourceLocation HEAD =
            ModelResourceLocation.standalone(EvansComputerMod.id("block/lidar_sensor_head"));
    /** Degrees per tick while scanning (one turn a second). */
    private static final float SPIN = 18f;

    public LidarSensorRenderer(BlockEntityRendererProvider.Context context) {
    }

    @Override
    public void render(LidarSensorBlockEntity be, float partialTick, PoseStack pose, MultiBufferSource buffers,
                       int light, int overlay) {
        var state = be.getBlockState();
        if(!(state.getBlock() instanceof LidarSensorBlock))
            return;
        var mc = Minecraft.getInstance();
        var model = mc.getModelManager().getModel(HEAD);
        Vec3 c = LidarSensorBlock.headCentre(state);
        float yaw = 180f - LidarSensorBlock.forward(state).toYRot();
        if(state.getValue(LidarSensorBlock.ACTIVE) && be.getLevel() != null)
            yaw += ((be.getLevel().getGameTime() % 20) + partialTick) * SPIN;

        pose.pushPose();
        // The head model is built around the block centre (8, 8, 8), facing north.
        pose.translate(c.x, c.y, c.z);
        pose.mulPose(Axis.YP.rotationDegrees(yaw));
        pose.translate(-0.5, -0.5, -0.5);
        mc.getBlockRenderer().getModelRenderer().renderModel(pose.last(), buffers.getBuffer(RenderType.cutout()),
                state, model, 1f, 1f, 1f, light, overlay);
        pose.popPose();
    }
}
//?}
