package com.example.evanscomputermod.computer.overlay.client;

//? if <=1.21.1 {

import com.example.evanscomputermod.block.ScreenBlockEntity;
import com.example.evanscomputermod.block.TerminalBlockEntity;
import com.example.evanscomputermod.computer.TerminalDisplay;
import com.example.evanscomputermod.computer.overlay.ItemOverlays;
import com.mojang.blaze3d.vertex.PoseStack;
import com.mojang.math.Axis;
import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.Font;
import net.minecraft.client.gui.GuiGraphics;
import net.minecraft.client.renderer.MultiBufferSource;
import net.minecraft.client.renderer.texture.OverlayTexture;
import net.minecraft.core.Direction;
import net.minecraft.world.item.ItemDisplayContext;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.phys.Vec3;

/**
 * Draws item overlays with Minecraft's own item renderer: in the terminal GUI
 * (with vanilla tooltips on hover) and flat on in-world Screen clusters.
 */
public final class ItemOverlayRenderer {

    private static final int FULL_BRIGHT = 0xF000F0;
    /** Screens farther than this don't draw items (blocks). */
    private static final double MAX_DISTANCE = 32;

    private ItemOverlayRenderer() {
    }

    /** GUI rectangle {x, y, size} of the first item drawn last frame (for the client check to hover it). */
    public static volatile float[] lastGuiItem;

    /**
     * Terminal GUI: the framebuffer is drawn at {@code (x0, y0)}, {@code w x h}
     * GUI pixels. Call after the framebuffer quad, outside its pose.
     */
    public static void renderGui(GuiGraphics gfx, Font font, TerminalBlockEntity te, int x0, int y0, int w, int h,
                                 int mouseX, int mouseY) {
        TerminalDisplay display = te.getDisplay();
        if (display.getDisplayMode() < 1) return;
        int gfxW = display.getGfxWidth(), gfxH = display.getGfxHeight();
        if (gfxW <= 0 || gfxH <= 0) return;
        ClientItemOverlays.Overlay o = ClientItemOverlays.get(te.getBlockPos(), ItemOverlays.TARGET_TERMINAL);
        if (o == null) return;
        float kx = w / (float) gfxW, ky = h / (float) gfxH;
        ItemStack hovered = null;
        boolean first = true;
        for (ItemOverlays.Entry e : o.entries) {
            ItemStack stack = o.stack(e);
            if (stack == null) continue;
            float gx = x0 + e.x() * kx, gy = y0 + e.y() * ky, gs = e.size() * Math.min(kx, ky);
            int cx0 = x0, cy0 = y0, cx1 = x0 + w, cy1 = y0 + h;
            if (e.clipW() > 0 && e.clipH() > 0) {
                cx0 = Math.max(cx0, x0 + (int) Math.floor(e.clipX() * kx));
                cy0 = Math.max(cy0, y0 + (int) Math.floor(e.clipY() * ky));
                cx1 = Math.min(cx1, x0 + (int) Math.ceil((e.clipX() + e.clipW()) * kx));
                cy1 = Math.min(cy1, y0 + (int) Math.ceil((e.clipY() + e.clipH()) * ky));
            }
            if (cx1 <= cx0 || cy1 <= cy0) continue;
            if (first) {
                lastGuiItem = new float[] {gx, gy, gs};
                first = false;
            }
            boolean dimmed = (e.flags() & ItemOverlays.FLAG_DIMMED) != 0;
            gfx.enableScissor(cx0, cy0, cx1, cy1);
            gfx.pose().pushPose();
            gfx.pose().translate(gx, gy, 0);
            gfx.pose().scale(gs / 16f, gs / 16f, 1f);
            gfx.renderItem(stack, 0, 0);
            gfx.renderItemDecorations(font, stack, 0, 0, e.label());
            if (dimmed) {
                gfx.pose().translate(0, 0, 300);
                gfx.fill(0, 0, 16, 16, 0xA0000000);
            }
            gfx.pose().popPose();
            gfx.disableScissor();
            if (!dimmed && mouseX >= Math.max(gx, cx0) && mouseX < Math.min(gx + gs, cx1)
                    && mouseY >= Math.max(gy, cy0) && mouseY < Math.min(gy + gs, cy1)) {
                hovered = stack;
            }
        }
        if (hovered != null) gfx.renderTooltip(font, hovered, mouseX, mouseY);
    }

    /**
     * In-world Screen cluster anchored at {@code be}: its picture spans
     * {@code cols x rows} blocks from the face's top-left corner
     * {@code (tlx, tly, tlz)} (block-local), going right along {@code (udx, 0, udz)}.
     */
    public static void renderScreen(ScreenBlockEntity be, PoseStack pose, MultiBufferSource buffers, Direction facing,
                                    int cols, int rows, float tlx, float tly, float tlz, float udx, float udz) {
        ClientItemOverlays.Overlay o = ClientItemOverlays.get(be.getOwnerTerminal(), ItemOverlays.TARGET_SCREEN);
        TerminalDisplay display = be.clientDisplay;
        if (o == null || display == null || be.getLevel() == null) return;
        int gfxW = display.getGfxWidth(), gfxH = display.getGfxHeight();
        if (gfxW <= 0 || gfxH <= 0) return;
        Minecraft mc = Minecraft.getInstance();
        Vec3 cam = mc.gameRenderer.getMainCamera().getPosition();
        if (cam.distanceToSqr(Vec3.atCenterOf(be.getBlockPos())) > MAX_DISTANCE * MAX_DISTANCE) return;

        float bx = cols / (float) gfxW, by = rows / (float) gfxH;
        float nx = facing.getStepX(), nz = facing.getStepZ();
        float angle = switch (facing) {
            case NORTH -> 180f;
            case EAST -> 90f;
            case WEST -> -90f;
            default -> 0f;
        };
        Font font = mc.font;
        for (ItemOverlays.Entry e : o.entries) {
            if ((e.flags() & ItemOverlays.FLAG_DIMMED) != 0) continue;
            ItemStack stack = o.stack(e);
            if (stack == null) continue;
            // In-world items can't be cut partway, so only fully visible ones are drawn.
            int cx0 = 0, cy0 = 0, cx1 = gfxW, cy1 = gfxH;
            if (e.clipW() > 0 && e.clipH() > 0) {
                cx0 = Math.max(cx0, e.clipX());
                cy0 = Math.max(cy0, e.clipY());
                cx1 = Math.min(cx1, e.clipX() + e.clipW());
                cy1 = Math.min(cy1, e.clipY() + e.clipH());
            }
            if (e.x() < cx0 || e.y() < cy0 || e.x() + e.size() > cx1 || e.y() + e.size() > cy1) continue;

            float s = e.size() * Math.min(bx, by);
            float cu = (e.x() + e.size() / 2f) * bx, cv = (e.y() + e.size() / 2f) * by;
            pose.pushPose();
            pose.translate(tlx + udx * cu + nx * 0.01f, tly - cv, tlz + udz * cu + nz * 0.01f);
            pose.mulPose(Axis.YP.rotationDegrees(angle));
            pose.scale(s, s, s * 0.04f);
            mc.getItemRenderer().renderStatic(stack, ItemDisplayContext.GUI, FULL_BRIGHT, OverlayTexture.NO_OVERLAY,
                    pose, buffers, be.getLevel(), 0);
            pose.popPose();

            if (!e.label().isEmpty()) {
                float ru = (e.x() + e.size()) * bx, rv = (e.y() + e.size()) * by;
                float t = s / 16f;
                pose.pushPose();
                pose.translate(tlx + udx * ru + nx * 0.015f, tly - rv, tlz + udz * ru + nz * 0.015f);
                pose.mulPose(Axis.YP.rotationDegrees(angle));
                pose.scale(t, -t, t);
                pose.translate(-font.width(e.label()), -8, 0);
                font.drawInBatch(e.label(), 0, 0, 0xFFFFFFFF, true, pose.last().pose(), buffers,
                        Font.DisplayMode.POLYGON_OFFSET, 0, FULL_BRIGHT);
                pose.popPose();
            }
        }
    }
}
//?}
