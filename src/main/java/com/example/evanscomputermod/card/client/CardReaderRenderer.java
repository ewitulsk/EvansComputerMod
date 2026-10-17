package com.example.evanscomputermod.card.client;

//? if <=1.21.1 {
import com.example.evanscomputermod.card.CardItem;
import com.example.evanscomputermod.card.CardKind;
import com.example.evanscomputermod.card.CardReaderBlock;
import com.example.evanscomputermod.card.CardReaderBlockEntity;
import com.mojang.blaze3d.vertex.PoseStack;
import com.mojang.math.Axis;
import net.minecraft.client.Minecraft;
import net.minecraft.client.renderer.MultiBufferSource;
import net.minecraft.client.renderer.blockentity.BlockEntityRenderer;
import net.minecraft.client.renderer.blockentity.BlockEntityRendererProvider;
import net.minecraft.core.Direction;
import net.minecraft.world.item.ItemDisplayContext;
import net.minecraft.world.item.ItemStack;

/**
 * Draws the card in a Card Reader/Writer: the real item (its kind's body and its painted
 * faces, from the block entity's client-safe copy), lying flat in the slot and sticking
 * out of the front, where the block model used to show a fixed SIM.
 */
public final class CardReaderRenderer implements BlockEntityRenderer<CardReaderBlockEntity> {
    /** Slot: 6 px wide, at y 9.5, from 3 px outside the front face inwards (models/card_reader.bbmodel's old card_sim). */
    private static final float SLOT_WIDTH = 6f, SLOT_Y = 9.5f, STICK_OUT = 3f;

    public CardReaderRenderer(BlockEntityRendererProvider.Context ctx) {}

    @Override
    public void render(CardReaderBlockEntity be, float partialTick, PoseStack pose, MultiBufferSource buffers, int light, int overlay) {
        ItemStack card = be.clientCard();
        CardKind kind = CardItem.kindOf(card);
        if (kind == null) return;
        Direction facing = be.getBlockState().hasProperty(CardReaderBlock.FACING) ? be.getBlockState().getValue(CardReaderBlock.FACING) : Direction.NORTH;
        CardItemRenderer.Geometry g = CardItemRenderer.GEOMETRY.get(kind);
        float s = SLOT_WIDTH / (g.x1() - g.x0());
        pose.pushPose();
        // The block's front to the north, as it was authored.
        pose.translate(0.5, 0, 0.5);
        pose.mulPose(Axis.YP.rotationDegrees(180 - facing.toYRot()));   // north = 0, west = 90, as the dock does
        pose.translate(-0.5, 0, -0.5);
        // Lay the card flat: its bottom edge at the slot's mouth, its length running into the block, front face up.
        pose.translate(8 / 16f, SLOT_Y / 16f, -STICK_OUT / 16f);
        pose.mulPose(Axis.XP.rotationDegrees(90));
        pose.scale(s, s, s);
        pose.translate(-(g.x0() + g.x1()) / 32f, -g.y0() / 16f, -(g.z0() + g.z1()) / 32f);
        Minecraft.getInstance().getItemRenderer().renderStatic(card, ItemDisplayContext.NONE, light, overlay, pose, buffers, be.getLevel(), 0);
        pose.popPose();
    }
}
//?}
