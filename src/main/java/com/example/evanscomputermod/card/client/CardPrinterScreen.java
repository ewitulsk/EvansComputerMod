package com.example.evanscomputermod.card.client;

//? if <=1.21.1 {
import com.example.evanscomputermod.card.printer.CardPrinterBlockEntity;
import com.example.evanscomputermod.card.printer.CardPrinterMenu;
import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.GuiGraphics;
import net.minecraft.client.gui.components.CycleButton;
import net.minecraft.client.gui.screens.inventory.AbstractContainerScreen;
import net.minecraft.network.chat.Component;
import net.minecraft.world.entity.player.Inventory;
import net.minecraft.world.inventory.Slot;

/**
 * Card Printer screen: the card in and out, the sixteen dye slots, the job's progress
 * and the Auto Eject setting. Printing itself is the computer's: a program sends the
 * design through the {@code card_printer} peripheral (the Card Designer app does).
 */
public class CardPrinterScreen extends AbstractContainerScreen<CardPrinterMenu> {
    private static final int BG = 0xFF2B3036, PANEL = 0xFF1B1F24, EDGE = 0xFF4A535C, SLOT = 0xFF0F1215, TEXT = 0xFFE6EDF3,
            DIM = 0xFF9AA6B2, GOOD = 0xFF7EE787, BAR = 0xFF58A6FF;

    private CycleButton<Boolean> autoEject;

    public CardPrinterScreen(CardPrinterMenu menu, Inventory inv, Component title) {
        super(menu, inv, title);
        imageWidth = 212;
        imageHeight = CardPrinterMenu.INV_Y + 82;
        inventoryLabelY = CardPrinterMenu.INV_Y - 11;
    }

    @Override
    protected void init() {
        super.init();
        autoEject = CycleButton.<Boolean>builder(v -> Component.literal(v ? "true" : "false"))
                .withValues(false, true)
                .withInitialValue(menu.autoEject())
                .create(leftPos + 62, topPos + 76, 144, 16, Component.translatable("gui.evanscomputermod.card_printer.auto_eject"),
                        (b, v) -> Minecraft.getInstance().gameMode.handleInventoryButtonClick(menu.containerId,
                                v ? CardPrinterMenu.BUTTON_AUTO_EJECT_TRUE : CardPrinterMenu.BUTTON_AUTO_EJECT_FALSE));
        addRenderableWidget(autoEject);
    }

    @Override
    protected void containerTick() {
        super.containerTick();
        if (autoEject != null && autoEject.getValue() != menu.autoEject()) autoEject.setValue(menu.autoEject());
    }

    @Override
    protected void renderBg(GuiGraphics g, float partialTick, int mouseX, int mouseY) {
        int x = leftPos, y = topPos;
        g.fill(x, y, x + imageWidth, y + imageHeight, EDGE);
        g.fill(x + 1, y + 1, x + imageWidth - 1, y + imageHeight - 1, BG);
        g.fill(x + 6, y + 14, x + 56, y + 74, PANEL);
        g.fill(x + 60, y + 56, x + imageWidth - 6, y + 72, PANEL);
        for (Slot s : menu.slots) {
            g.fill(x + s.x - 1, y + s.y - 1, x + s.x + 17, y + s.y + 17, EDGE);
            g.fill(x + s.x, y + s.y, x + s.x + 16, y + s.y + 16, SLOT);
        }
        // the head: a bar between the slots that fills while printing
        int p = menu.progress();
        g.fill(x + 10, y + 40, x + 52, y + 44, PANEL);
        if (p >= 0) g.fill(x + 10, y + 40, x + 10 + 42 * Math.min(p, CardPrinterBlockEntity.PRINT_TICKS) / CardPrinterBlockEntity.PRINT_TICKS, y + 44, BAR);
    }

    @Override
    protected void renderLabels(GuiGraphics g, int mouseX, int mouseY) {
        g.drawString(font, title, titleLabelX, titleLabelY, TEXT, false);
        g.drawString(font, playerInventoryTitle, inventoryLabelX, inventoryLabelY, DIM, false);
        String state = menu.progress() >= 0 ? "printing" : menu.kindOut() != null ? "done" : menu.kindIn() != null ? "ready" : "empty";
        g.drawString(font, Component.translatable("gui.evanscomputermod.card_printer.state." + state), 64, 60,
                "done".equals(state) ? GOOD : TEXT, false);
    }

    @Override
    public void render(GuiGraphics g, int mouseX, int mouseY, float partialTick) {
        super.render(g, mouseX, mouseY, partialTick);
        renderTooltip(g, mouseX, mouseY);
    }
}
//?}
