package com.example.evanscomputermod.phone.client;

//? if <=1.21.1 {
import com.example.evanscomputermod.phone.ChargingDockBlock;
import com.example.evanscomputermod.phone.ChargingDockBlockEntity;
import com.mojang.blaze3d.vertex.PoseStack;
import com.mojang.math.Axis;
import net.minecraft.client.renderer.MultiBufferSource;
import net.minecraft.client.renderer.blockentity.BlockEntityRenderer;
import net.minecraft.client.renderer.blockentity.BlockEntityRendererProvider;
import net.minecraft.client.renderer.texture.OverlayTexture;
import net.minecraft.world.item.ItemStack;

/**
 * Draws the docked phone standing in the Charging Dock, leaning on the back
 * rest with its screen to the front: dark glass with a charge bar (a docked
 * phone is off).
 */
public class ChargingDockRenderer implements BlockEntityRenderer<ChargingDockBlockEntity> {

    public ChargingDockRenderer(BlockEntityRendererProvider.Context ctx) {
    }

    @Override
    public void render(ChargingDockBlockEntity be, float partialTick, PoseStack pose, MultiBufferSource buffers, int light, int overlay) {
        ItemStack phone = be.phone();
        if (phone.isEmpty()) return;
        var state = be.getBlockState();
        float yRot = switch (state.getValue(ChargingDockBlock.FACING)) {
            case SOUTH -> 180;
            case EAST -> 270;
            case WEST -> 90;
            default -> 0;
        };
        pose.pushPose();
        pose.translate(0.5, 0, 0.5);
        pose.mulPose(Axis.YP.rotationDegrees(yRot));
        pose.translate(-0.5, 0, -0.5);
        // Stand it in the cradle (the model's phone spans y 1..13.5, z 7.5..8.5), leaning back 10 degrees.
        pose.translate(0, 1.5 / 16, 2.2 / 16);
        pose.translate(0.5, 0, 0.5);
        pose.mulPose(Axis.XP.rotationDegrees(10));
        pose.translate(-0.5, 0, -0.5);
        PhoneItemRenderer.renderBody(phone, pose, buffers, light, OverlayTexture.NO_OVERLAY);
        PhoneItemRenderer.drawScreen(PhoneItemRenderer.docked(phone, state.getValue(ChargingDockBlock.ACTIVE)), pose, buffers);
        pose.popPose();
    }
}
//?}
