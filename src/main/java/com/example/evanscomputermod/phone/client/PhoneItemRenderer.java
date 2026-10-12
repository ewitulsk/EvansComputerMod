package com.example.evanscomputermod.phone.client;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.computer.display.PanelTier;
import com.example.evanscomputermod.phone.PhoneConfig;
import com.example.evanscomputermod.phone.PhoneHardware;
import com.example.evanscomputermod.phone.PhoneHost;
import com.example.evanscomputermod.phone.PhoneItemData;
import com.example.evanscomputermod.phone.PhoneState;
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
import org.jetbrains.annotations.Nullable;

import java.util.UUID;

/**
 * Draws the ePhone item everywhere it appears (first and third person in
 * either hand, the inventory, item frames, the ground, the Charging Dock):
 * the Blockbench body ({@code models/item/ephone_body.json}) and, over its
 * glass, the phone's screen.
 *
 * <p>What the screen shows: your own phone shows its display (display id 0,
 * streamed to the holder) while it is awake, and is dark asleep or off.
 * Another player's phone shows what the server lets onlookers see (the
 * {@code Status} packet): the mirror (display id {@link PhoneHost#DISPLAY_MIRROR})
 * at FULL, a lit glass at GLOW, dark at OFF, and always dark when you have
 * "render other players' phones" off.
 */
public final class PhoneItemRenderer extends BlockEntityWithoutLevelRenderer {

    public static final ModelResourceLocation BODY =
            ModelResourceLocation.standalone(ResourceLocation.fromNamespaceAndPath(EvansComputerMod.MODID, "item/ephone_body"));
    private static final ResourceLocation SOLID = ResourceLocation.fromNamespaceAndPath(EvansComputerMod.MODID, "textures/block/screen_solid.png");
    private static final int FULL_BRIGHT = 0xF000F0;

    /** The glass the screen is drawn on, in model pixels (north face, z = SCREEN_Z). */
    public static final float SCREEN_X0 = 5f, SCREEN_X1 = 11f, SCREEN_Y0 = 2.5f, SCREEN_Y1 = 12.5f, SCREEN_Z = 7.49f;
    private static final int GLOW = 0xFF5A7896;

    /** What the last first-person draw of a phone showed (render checks). */
    public static volatile String lastFirstPerson = "", lastOffHand = "";

    public PhoneItemRenderer() {
        super(Minecraft.getInstance().getBlockEntityRenderDispatcher(), Minecraft.getInstance().getEntityModels());
    }

    @Override
    public void renderByItem(ItemStack stack, ItemDisplayContext ctx, PoseStack pose, MultiBufferSource buffers, int light, int overlay) {
        renderBody(stack, pose, buffers, light, overlay);
        Screen s = screenFor(stack, ctx);
        drawScreen(s, pose, buffers);
        if (ctx == ItemDisplayContext.FIRST_PERSON_RIGHT_HAND) lastFirstPerson = s.describe();
        else if (ctx == ItemDisplayContext.FIRST_PERSON_LEFT_HAND) lastOffHand = s.describe();
    }

    static void renderBody(ItemStack stack, PoseStack pose, MultiBufferSource buffers, int light, int overlay) {
        Minecraft mc = Minecraft.getInstance();
        BakedModel body = mc.getModelManager().getModel(BODY);
        VertexConsumer vc = ItemRenderer.getFoilBufferDirect(buffers, Sheets.cutoutBlockSheet(), true, stack.hasFoil());
        mc.getItemRenderer().renderModelLists(body, stack, light, overlay, pose, vc);
    }

    /** What to draw on a phone's glass. */
    record Screen(@Nullable ResourceLocation texture, int color, float battery, String why) {
        static Screen dark(String why) {
            return new Screen(null, 0, -1, why);
        }

        String describe() {
            return texture != null ? "display:" + why : color != 0 ? "glow:" + why : "dark:" + why;
        }
    }

    static Screen screenFor(ItemStack stack, ItemDisplayContext ctx) {
        UUID id = PhoneItemData.id(stack);
        if (id == null) return Screen.dark("new");
        PhoneHardware hw = PhoneHardware.of(stack);
        PanelTier tier = hw.tier();
        if (tier == null) return Screen.dark("no panel");
        PhoneClient.Status st = PhoneClient.status(id);
        boolean own = ctx.firstPerson() || ctx == ItemDisplayContext.GUI || PhoneClient.isLocalPhone(id);
        if (own) {
            boolean awake = st != null ? st.state() == PhoneState.AWAKE : ctx.firstPerson();
            if (!awake) return Screen.dark(st == null ? "no status" : st.state().id());
            PhoneClient.wantFrames(id, 0);
            ResourceLocation tex = PhoneScreenTexture.get(id, 0, tier, hw.tone());
            return tex != null ? new Screen(tex, 0, -1, "own") : new Screen(null, GLOW, -1, "waiting");
        }
        if (!PhoneClient.renderOthers()) return Screen.dark("render others off");
        if (st == null) return Screen.dark("no status");
        return switch (st.visibility()) {
            case OFF -> Screen.dark("visibility off");
            case GLOW -> new Screen(null, GLOW, -1, "glow");
            case FULL -> {
                PhoneClient.wantFrames(id, PhoneHost.DISPLAY_MIRROR);
                ResourceLocation tex = PhoneScreenTexture.get(id, PhoneHost.DISPLAY_MIRROR, tier, hw.tone());
                yield tex != null ? new Screen(tex, 0, -1, "mirror") : new Screen(null, GLOW, -1, "mirror waiting");
            }
        };
    }

    /** The screen of a docked phone: dark glass with a charge bar. */
    static Screen docked(ItemStack stack, boolean charging) {
        return new Screen(null, 0, (float) (PhoneItemData.batteryPercent(stack) / 100.0), charging ? "charging" : "docked");
    }

    static void drawScreen(Screen s, PoseStack pose, MultiBufferSource buffers) {
        if (s.texture() != null) {
            quad(pose, buffers.getBuffer(RenderType.entityTranslucentEmissive(s.texture())), SCREEN_X0, SCREEN_Y0, SCREEN_X1, SCREEN_Y1,
                    0xFFFFFFFF, true);
        } else if (s.color() != 0) {
            quad(pose, buffers.getBuffer(RenderType.entityTranslucentEmissive(SOLID)), SCREEN_X0, SCREEN_Y0, SCREEN_X1, SCREEN_Y1,
                    s.color(), false);
        }
        if (s.battery() >= 0) {
            // Charge bar: an outline and a fill in the middle of the glass.
            float x0 = 6.5f, x1 = 9.5f, y0 = 6.5f, y1 = 8.0f;
            VertexConsumer vc = buffers.getBuffer(RenderType.entityTranslucentEmissive(SOLID));
            quad(pose, vc, x0, y0, x1, y1, 0xFF30363D, false);
            float fill = x0 + 0.15f + (x1 - x0 - 0.3f) * Math.max(0, Math.min(1, s.battery()));
            int c = s.battery() > 0.5f ? 0xFF3FB950 : s.battery() > 0.2f ? 0xFFD29922 : 0xFFF85149;
            quadAt(pose, vc, x0 + 0.15f, y0 + 0.15f, fill, y1 - 0.15f, SCREEN_Z - 0.01f, c, false);
            quadAt(pose, vc, x1, 7.0f, x1 + 0.3f, 7.5f, SCREEN_Z - 0.01f, 0xFF30363D, false);
        }
    }

    private static void quad(PoseStack pose, VertexConsumer vc, float x0, float y0, float x1, float y1, int argb, boolean uv) {
        quadAt(pose, vc, x0, y0, x1, y1, SCREEN_Z, argb, uv);
    }

    /**
     * A quad on the phone's front (north, -Z) at depth {@code z}, in model
     * pixels. Seen from the front, +x runs to the viewer's left, so u = 0 is at x1.
     */
    private static void quadAt(PoseStack pose, VertexConsumer vc, float x0, float y0, float x1, float y1, float z, int argb, boolean uv) {
        PoseStack.Pose p = pose.last();
        float s = 1 / 16f;
        float ax = x1 * s, bx = x0 * s, ty = y1 * s, by = y0 * s, zz = z * s;
        // Counter-clockwise as seen from the front: top-left, bottom-left, bottom-right, top-right.
        vc.addVertex(p, ax, ty, zz).setColor(argb).setUv(0, 0).setOverlay(OverlayTexture.NO_OVERLAY).setLight(FULL_BRIGHT).setNormal(p, 0, 0, -1);
        vc.addVertex(p, ax, by, zz).setColor(argb).setUv(0, 1).setOverlay(OverlayTexture.NO_OVERLAY).setLight(FULL_BRIGHT).setNormal(p, 0, 0, -1);
        vc.addVertex(p, bx, by, zz).setColor(argb).setUv(1, 1).setOverlay(OverlayTexture.NO_OVERLAY).setLight(FULL_BRIGHT).setNormal(p, 0, 0, -1);
        vc.addVertex(p, bx, ty, zz).setColor(argb).setUv(1, 0).setOverlay(OverlayTexture.NO_OVERLAY).setLight(FULL_BRIGHT).setNormal(p, 0, 0, -1);
    }
}
//?}
