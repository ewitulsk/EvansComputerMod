package com.example.evanscomputermod.storage.client;

//? if <=1.21.1 {

import com.example.evanscomputermod.storage.device.DriveBlockEntity;
import com.example.evanscomputermod.storage.device.DriveMenu;
import net.minecraft.client.gui.GuiGraphics;
import net.minecraft.client.gui.screens.inventory.AbstractContainerScreen;
import net.minecraft.network.chat.Component;
import net.minecraft.world.entity.player.Inventory;
import net.minecraft.world.inventory.Slot;

/** Drive GUI, drawn with plain fills (no texture): ten cell slots over the player inventory. */
public class DriveScreen extends AbstractContainerScreen<DriveMenu> {

    private static final int PANEL = 0xFFC6C6C6;
    private static final int LIGHT = 0xFFFFFFFF;
    private static final int DARK = 0xFF555555;
    private static final int SLOT_BG = 0xFF8B8B8B;
    private static final int DUPLICATE = 0x80FF2020;

    public DriveScreen(DriveMenu menu, Inventory inventory, Component title) {
        super(menu, inventory, title);
        imageWidth = 176;
        imageHeight = 166;
        inventoryLabelY = DriveMenu.INV_Y - 11;
    }

    @Override
    public void render(GuiGraphics gfx, int mouseX, int mouseY, float partialTick) {
        super.render(gfx, mouseX, mouseY, partialTick);
        renderTooltip(gfx, mouseX, mouseY);
    }

    @Override
    protected void renderBg(GuiGraphics gfx, float partialTick, int mouseX, int mouseY) {
        int x = leftPos, y = topPos;
        gfx.fill(x, y, x + imageWidth, y + imageHeight, PANEL);
        gfx.fill(x, y, x + imageWidth - 1, y + 1, LIGHT);
        gfx.fill(x, y, x + 1, y + imageHeight - 1, LIGHT);
        gfx.fill(x + 1, y + imageHeight - 1, x + imageWidth, y + imageHeight, DARK);
        gfx.fill(x + imageWidth - 1, y + 1, x + imageWidth, y + imageHeight, DARK);
        DriveBlockEntity drive = menu.drive();
        for (Slot slot : menu.slots) {
            int sx = x + slot.x - 1, sy = y + slot.y - 1;
            gfx.fill(sx, sy, sx + 18, sy + 18, DARK);
            gfx.fill(sx + 1, sy + 1, sx + 18, sy + 18, LIGHT);
            gfx.fill(sx + 1, sy + 1, sx + 17, sy + 17, SLOT_BG);
            if (drive != null && slot.index < DriveBlockEntity.SLOTS
                    && drive.clientStatus(slot.index) == DriveBlockEntity.STATUS_DUPLICATE) {
                gfx.fill(sx + 1, sy + 1, sx + 17, sy + 17, DUPLICATE);
            }
        }
    }

    @Override
    protected void renderTooltip(GuiGraphics gfx, int x, int y) {
        DriveBlockEntity drive = menu.drive();
        if (hoveredSlot != null && drive != null && hoveredSlot.index < DriveBlockEntity.SLOTS
                && drive.clientStatus(hoveredSlot.index) == DriveBlockEntity.STATUS_DUPLICATE
                && menu.getCarried().isEmpty()) {
            gfx.renderComponentTooltip(font, java.util.List.of(
                    hoveredSlot.getItem().getHoverName(),
                    Component.translatable("tooltip.evanscomputermod.cell.duplicate")), x, y);
            return;
        }
        super.renderTooltip(gfx, x, y);
    }
}
//?}
