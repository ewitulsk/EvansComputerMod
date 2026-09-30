package com.example.evanscomputermod.storage.client;

//? if <=1.21.1 {

import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.storage.StorageContent;
import com.example.evanscomputermod.storage.core.CellTier;
import com.example.evanscomputermod.storage.device.DriveBlockEntity;
import com.example.evanscomputermod.storage.item.CellSummary;
import com.example.evanscomputermod.storage.item.StorageCellItem;
import com.mojang.blaze3d.vertex.PoseStack;
import com.mojang.blaze3d.vertex.VertexConsumer;
import net.minecraft.client.renderer.MultiBufferSource;
import net.minecraft.client.renderer.RenderType;
import net.minecraft.client.renderer.blockentity.BlockEntityRenderer;
import net.minecraft.client.renderer.blockentity.BlockEntityRendererProvider;
import net.minecraft.client.renderer.texture.OverlayTexture;
import net.minecraft.core.Direction;
import net.minecraft.resources.Identifier;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.block.HorizontalDirectionalBlock;

/**
 * Draws the cells sitting in a Drive's front slots: a tier-coloured cell face
 * per occupied slot and an LED (green / amber when over 75% full / red when
 * full or refused as a duplicate). Flat quads on the front face, no models.
 */
public class DriveRenderer implements BlockEntityRenderer<DriveBlockEntity> {

    private static final Identifier WHITE = EvansComputerMod.id("textures/block/screen_solid.png");
    private static final RenderType TYPE = RenderType.entityCutout(WHITE);
    private static final int FULL_BRIGHT = 0xF000F0;

    /** Slot rectangles on the front face, pixels from the viewer's top-left. */
    static final float[] COL_X = {1.5f, 4.25f, 7f, 9.75f, 12.5f};
    static final float SLOT_W = 2.25f;
    static final float[] ROW_Y = {2f, 8.5f};
    static final float SLOT_H = 5.5f;

    public DriveRenderer(BlockEntityRendererProvider.Context context) {
    }

    private static int tierColor(CellTier tier) {
        return switch (tier) {
            case K1 -> 0xFFB8B8B8;
            case K4 -> 0xFFE0B040;
            case K16 -> 0xFF40C0D0;
            case K64 -> 0xFFB060E0;
        };
    }

    @Override
    public void render(DriveBlockEntity be, float partialTick, PoseStack pose, MultiBufferSource buffers,
                       int light, int overlay) {
        Direction facing = be.getBlockState().getValue(HorizontalDirectionalBlock.FACING);
        Face face = Face.of(facing);
        VertexConsumer buf = buffers.getBuffer(TYPE);
        PoseStack.Pose p = pose.last();
        for (int slot = 0; slot < DriveBlockEntity.SLOTS; slot++) {
            ItemStack stack = be.cellHandler().getStackInSlot(slot);
            CellTier tier = StorageCellItem.tierOf(stack);
            if (tier == null) continue;
            float u0 = COL_X[slot % 5], v0 = ROW_Y[slot / 5];
            float u1 = u0 + SLOT_W, v1 = v0 + SLOT_H;
            face.quad(p, buf, u0, v0, u1, v1, 0.002f, 0xFF202226, light);
            face.quad(p, buf, u0 + 0.25f, v0 + 1.5f, u1 - 0.25f, v1 - 0.25f, 0.004f, tierColor(tier), light);
            face.quad(p, buf, u0 + 0.75f, v1 - 1.5f, u1 - 0.75f, v1 - 0.75f, 0.006f, 0xFF303030, light);
            CellSummary s = stack.get(StorageContent.CELL_SUMMARY.get());
            double fill = s == null ? 0 : Math.max((double) s.bytes() / tier.bytes(), s.types() / (double) CellTier.MAX_TYPES);
            int led;
            if (be.clientStatus(slot) == DriveBlockEntity.STATUS_DUPLICATE || fill >= 0.999) led = 0xFFFF3030;
            else if (fill >= 0.75) led = 0xFFFFB020;
            else led = 0xFF40FF60;
            face.quad(p, buf, u0 + 0.6f, v0 + 0.4f, u1 - 0.6f, v0 + 1.1f, 0.006f, led, FULL_BRIGHT);
        }
    }

    /** A horizontal block face, addressed in pixels from the viewer's top-left. */
    record Face(float tlx, float tlz, float udx, float udz, float nx, float nz) {
        static Face of(Direction facing) {
            return switch (facing) {
                case SOUTH -> new Face(0, 1, 1, 0, 0, 1);
                case WEST -> new Face(0, 0, 0, 1, -1, 0);
                case EAST -> new Face(1, 1, 0, -1, 1, 0);
                default -> new Face(1, 0, -1, 0, 0, -1);
            };
        }

        void quad(PoseStack.Pose pose, VertexConsumer buf, float u0, float v0, float u1, float v1, float eps,
                  int color, int light) {
            float a0 = u0 / 16f, a1 = u1 / 16f, y0 = 1 - v0 / 16f, y1 = 1 - v1 / 16f;
            float ex = nx * eps, ez = nz * eps;
            vertex(pose, buf, tlx + udx * a0 + ex, y0, tlz + udz * a0 + ez, color, light);
            vertex(pose, buf, tlx + udx * a0 + ex, y1, tlz + udz * a0 + ez, color, light);
            vertex(pose, buf, tlx + udx * a1 + ex, y1, tlz + udz * a1 + ez, color, light);
            vertex(pose, buf, tlx + udx * a1 + ex, y0, tlz + udz * a1 + ez, color, light);
        }

        private void vertex(PoseStack.Pose pose, VertexConsumer buf, float x, float y, float z, int color, int light) {
            buf.addVertex(pose, x, y, z).setColor(color).setUv(0.5f, 0.5f).setOverlay(OverlayTexture.NO_OVERLAY)
                    .setLight(light).setNormal(pose, nx, 0, nz);
        }
    }
}
//?}
