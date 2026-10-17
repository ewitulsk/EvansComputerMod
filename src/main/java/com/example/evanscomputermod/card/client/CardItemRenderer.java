package com.example.evanscomputermod.card.client;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.card.CardItem;
import com.example.evanscomputermod.card.CardKind;
import com.example.evanscomputermod.card.Stripe;
import com.mojang.blaze3d.vertex.PoseStack;
import com.mojang.blaze3d.vertex.VertexConsumer;
import net.minecraft.client.Minecraft;
import net.minecraft.client.renderer.BlockEntityWithoutLevelRenderer;
import net.minecraft.client.renderer.MultiBufferSource;
import net.minecraft.client.renderer.RenderType;
import net.minecraft.client.renderer.Sheets;
import net.minecraft.client.renderer.entity.ItemRenderer;
import net.minecraft.client.renderer.texture.OverlayTexture;
import net.minecraft.client.resources.model.BakedModel;
import net.minecraft.client.resources.model.ModelResourceLocation;
import net.minecraft.resources.ResourceLocation;
import net.minecraft.world.item.ItemDisplayContext;
import net.minecraft.world.item.ItemStack;

import java.util.EnumMap;
import java.util.Map;

/**
 * Draws every card item: the kind's Blockbench body ({@code models/item/card_<kind>_body.json}:
 * the slab's edges, the chip pad and the stripe) and, on its two large faces, the card's
 * painted faces from {@link CardFaceTexture}. The body is authored north-facing with the
 * front on the north (-Z) face; the geometry below must match the .bbmodel (models/card_*.bbmodel).
 */
public final class CardItemRenderer extends BlockEntityWithoutLevelRenderer {

    /** Where a kind's faces are, in model pixels: x0, y0, x1, y1 of the face rectangle and the slab's z range. */
    public record Geometry(float x0, float y0, float x1, float y1, float z0, float z1) {}

    public static final Map<CardKind, Geometry> GEOMETRY = new EnumMap<>(CardKind.class);
    public static final Map<CardKind, ModelResourceLocation> BODIES = new EnumMap<>(CardKind.class);

    static {
        Geometry landscape = new Geometry(1f, 3.625f, 15f, 12.375f, 7.75f, 8.25f);
        GEOMETRY.put(CardKind.BANK, landscape);
        GEOMETRY.put(CardKind.NFC, landscape);
        GEOMETRY.put(CardKind.HYBRID, landscape);
        GEOMETRY.put(CardKind.SIM, new Geometry(4f, 1f, 12f, 14f, 7.75f, 8.25f));
        for (CardKind k : CardKind.values())
            BODIES.put(k, ModelResourceLocation.standalone(ResourceLocation.fromNamespaceAndPath(EvansComputerMod.MODID, "item/card_" + k.id() + "_body")));
    }

    /** What the last first-person draw showed (render checks): kind and faces hash. */
    public static volatile String lastFirstPerson = "", lastGui = "";

    public CardItemRenderer() {
        super(Minecraft.getInstance().getBlockEntityRenderDispatcher(), Minecraft.getInstance().getEntityModels());
    }

    @Override
    public void renderByItem(ItemStack stack, ItemDisplayContext ctx, PoseStack pose, MultiBufferSource buffers, int light, int overlay) {
        CardKind kind = CardItem.kindOf(stack);
        if (kind == null) return;
        Minecraft mc = Minecraft.getInstance();
        BakedModel body = mc.getModelManager().getModel(BODIES.get(kind));
        VertexConsumer vc = ItemRenderer.getFoilBufferDirect(buffers, Sheets.cutoutBlockSheet(), true, stack.hasFoil());
        mc.getItemRenderer().renderModelLists(body, stack, light, overlay, pose, vc);

        Stripe s = CardItem.stripe(stack);
        ResourceLocation tex = CardFaceTexture.get(kind, CardItem.faces(stack), s != null && s.corrupted());
        Geometry g = GEOMETRY.get(kind);
        VertexConsumer fv = buffers.getBuffer(RenderType.entityCutoutNoCull(tex));
        face(pose, fv, g, true, light);
        face(pose, fv, g, false, light);
        String what = kind.id() + ":" + (CardItem.faces(stack) == null ? "blank" : Long.toHexString(CardItem.faces(stack).contentHash()));
        if (ctx == ItemDisplayContext.FIRST_PERSON_RIGHT_HAND || ctx == ItemDisplayContext.FIRST_PERSON_LEFT_HAND) lastFirstPerson = what;
        else if (ctx == ItemDisplayContext.GUI) lastGui = what;
    }

    /**
     * One face of the slab. The front is the north face (-Z), seen from the front +x runs
     * to the viewer's left, so u = 0 is at x1; the back (+Z) is seen the other way round,
     * so the same art reads correctly (not mirrored) when the card is turned over.
     * The texture holds the front in v 0..0.5 and the back in v 0.5..1.
     */
    private static void face(PoseStack pose, VertexConsumer vc, Geometry g, boolean front, int light) {
        PoseStack.Pose p = pose.last();
        float s = 1 / 16f;
        float z = (front ? g.z0() - 0.01f : g.z1() + 0.01f) * s;
        float lx = g.x0() * s, rx = g.x1() * s, ty = g.y1() * s, by = g.y0() * s;
        float v0 = front ? 0f : 0.5f, v1 = front ? 0.5f : 1f;
        float nz = front ? -1 : 1;
        if (front) {
            vertex(vc, p, rx, ty, z, 0, v0, light, nz);
            vertex(vc, p, rx, by, z, 0, v1, light, nz);
            vertex(vc, p, lx, by, z, 1, v1, light, nz);
            vertex(vc, p, lx, ty, z, 1, v0, light, nz);
        } else {
            vertex(vc, p, lx, ty, z, 0, v0, light, nz);
            vertex(vc, p, lx, by, z, 0, v1, light, nz);
            vertex(vc, p, rx, by, z, 1, v1, light, nz);
            vertex(vc, p, rx, ty, z, 1, v0, light, nz);
        }
    }

    private static void vertex(VertexConsumer vc, PoseStack.Pose p, float x, float y, float z, float u, float v, int light, float nz) {
        vc.addVertex(p, x, y, z).setColor(0xFFFFFFFF).setUv(u, v).setOverlay(OverlayTexture.NO_OVERLAY).setLight(light).setNormal(p, 0, 0, nz);
    }
}
//?}
