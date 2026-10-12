package com.example.evanscomputermod.phone.client;

//? if <=1.21.1 {
import com.example.evanscomputermod.phone.PhoneClientConfig;
import com.example.evanscomputermod.phone.PhoneMenu;
import net.minecraft.client.gui.GuiGraphics;
import net.minecraft.client.gui.components.Button;
import net.minecraft.client.gui.screens.inventory.AbstractContainerScreen;
import net.minecraft.network.chat.Component;
import net.minecraft.world.entity.player.Inventory;

/**
 * The phone's slot screen: a phone-shaped panel with the display panel slot
 * on top, the two module slots and the SIM tray, the player's inventory below,
 * and the two visibility toggles of this player (share my screen, render
 * other players' phones), saved to the client config and sent to the server.
 */
public class PhoneMenuScreen extends AbstractContainerScreen<PhoneMenu> {
    private static final String[] LABELS = {"gui.evanscomputermod.ephone.slot.panel", "gui.evanscomputermod.ephone.slot.module",
            "gui.evanscomputermod.ephone.slot.module", "gui.evanscomputermod.ephone.slot.sim"};
    private Button share, others;

    public PhoneMenuScreen(PhoneMenu menu, Inventory inv, Component title) {
        super(menu, inv, title);
        imageWidth = 176;
        imageHeight = 186;
        inventoryLabelY = imageHeight - 94;
    }

    @Override
    protected void init() {
        super.init();
        share = addRenderableWidget(Button.builder(shareLabel(), b -> {
            PhoneClientConfig.SHARE_SCREEN.set(!PhoneClientConfig.share());
            PhoneClientConfig.SPEC.save();
            PhoneClient.sendPrefs();
            b.setMessage(shareLabel());
        }).bounds(leftPos + imageWidth + 4, topPos + 8, 120, 20).build());
        others = addRenderableWidget(Button.builder(othersLabel(), b -> {
            PhoneClientConfig.RENDER_OTHERS.set(!PhoneClientConfig.renderOthers());
            PhoneClientConfig.SPEC.save();
            PhoneClient.sendPrefs();
            b.setMessage(othersLabel());
        }).bounds(leftPos + imageWidth + 4, topPos + 32, 120, 20).build());
    }

    private static Component onOff(boolean on) {
        return Component.translatable(on ? "options.on" : "options.off");
    }

    private static Component shareLabel() {
        return Component.translatable("gui.evanscomputermod.ephone.share", onOff(PhoneClientConfig.share()));
    }

    private static Component othersLabel() {
        return Component.translatable("gui.evanscomputermod.ephone.render_others", onOff(PhoneClientConfig.renderOthers()));
    }

    @Override
    protected void renderBg(GuiGraphics g, float partialTick, int mouseX, int mouseY) {
        int x = leftPos, y = topPos;
        // Phone body (top) and an inventory tray.
        g.fill(x + 40, y + 2, x + 136, y + 100, 0xFF1B1F24);
        g.fill(x + 42, y + 4, x + 134, y + 98, 0xFF2D333B);
        g.fill(x + 74, y + 7, x + 102, y + 9, 0xFF0D1117);
        g.fill(x, y + 98, x + imageWidth, y + imageHeight, 0xFFC6C6C6);
        g.fill(x + 1, y + 99, x + imageWidth - 1, y + imageHeight - 1, 0xFFB0B0B0);
        for (int i = 0; i < PhoneMenu.SLOTS; i++) slotFrame(g, x + PhoneMenu.POS[i][0], y + PhoneMenu.POS[i][1], 0xFF0D1117, 0xFF57606A);
        for (int row = 0; row < 3; row++)
            for (int col = 0; col < 9; col++) slotFrame(g, x + 8 + col * 18, y + 104 + row * 18, 0xFF8B8B8B, 0xFF373737);
        for (int col = 0; col < 9; col++) slotFrame(g, x + 8 + col * 18, y + 162, 0xFF8B8B8B, 0xFF373737);
    }

    private static void slotFrame(GuiGraphics g, int sx, int sy, int inner, int border) {
        g.fill(sx - 1, sy - 1, sx + 17, sy + 17, border);
        g.fill(sx, sy, sx + 16, sy + 16, inner);
    }

    @Override
    protected void renderLabels(GuiGraphics g, int mouseX, int mouseY) {
        for (int i = 0; i < PhoneMenu.SLOTS; i++) {
            Component c = Component.translatable(LABELS[i]);
            int cx = PhoneMenu.POS[i][0] + 8;
            g.drawString(font, c, cx - font.width(c) / 2, PhoneMenu.POS[i][1] + 19, 0xFF8B949E, false);
        }
        g.drawString(font, playerInventoryTitle, 8, inventoryLabelY + 4, 0xFF404040, false);
    }

    @Override
    public void render(GuiGraphics g, int mouseX, int mouseY, float partialTick) {
        super.render(g, mouseX, mouseY, partialTick);
        renderTooltip(g, mouseX, mouseY);
    }
}
//?}
