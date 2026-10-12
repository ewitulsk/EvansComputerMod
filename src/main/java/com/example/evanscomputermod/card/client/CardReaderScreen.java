package com.example.evanscomputermod.card.client;

//? if <=1.21.1 {
import com.example.evanscomputermod.card.CardReaderMenu;
import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.GuiGraphics;
import net.minecraft.client.gui.components.CycleButton;
import net.minecraft.client.gui.screens.inventory.AbstractContainerScreen;
import net.minecraft.network.chat.Component;
import net.minecraft.world.entity.player.Inventory;
import net.minecraft.world.inventory.Slot;

/**
 * Card Reader/Writer screen: the card slot, what is in it (no card / blank SIM /
 * written SIM, held for the computer or released) and the "Auto Eject Sim" setting
 * (true / false).
 */
public class CardReaderScreen extends AbstractContainerScreen<CardReaderMenu> {
    private static final int BG = 0xFF2B3036, PANEL = 0xFF1B1F24, EDGE = 0xFF4A535C, SLOT = 0xFF0F1215, TEXT = 0xFFE6EDF3,
            DIM = 0xFF9AA6B2, GOOD = 0xFF7EE787;

    private CycleButton<Boolean> autoEject;

    public CardReaderScreen(CardReaderMenu menu, Inventory inv, Component title) {
        super(menu, inv, title);
        imageWidth = 176;
        imageHeight = 166;
        inventoryLabelY = imageHeight - 94;
    }

    @Override
    protected void init() {
        super.init();
        autoEject = CycleButton.<Boolean>builder(v -> Component.literal(v ? "true" : "false"))
                .withValues(false, true)
                .withInitialValue(menu.autoEject())
                .create(leftPos + 8, topPos + 56, 160, 16, Component.translatable("gui.evanscomputermod.card_reader.auto_eject"),
                        (b, v) -> Minecraft.getInstance().gameMode.handleInventoryButtonClick(menu.containerId,
                                v ? CardReaderMenu.BUTTON_AUTO_EJECT_TRUE : CardReaderMenu.BUTTON_AUTO_EJECT_FALSE));
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
        g.fill(x + 6, y + 16, x + imageWidth - 6, y + 52, PANEL);
        for (Slot s : menu.slots) {
            g.fill(x + s.x - 1, y + s.y - 1, x + s.x + 17, y + s.y + 17, EDGE);
            g.fill(x + s.x, y + s.y, x + s.x + 16, y + s.y + 16, SLOT);
        }
    }

    @Override
    protected void renderLabels(GuiGraphics g, int mouseX, int mouseY) {
        g.drawString(font, title, titleLabelX, titleLabelY, TEXT, false);
        g.drawString(font, playerInventoryTitle, inventoryLabelX, inventoryLabelY, DIM, false);
        String state = switch (menu.cardState()) {
            case 0 -> "no_card";
            case 1 -> "blank";
            case 2 -> "written";
            default -> "other";
        };
        g.drawString(font, Component.translatable("gui.evanscomputermod.card_reader.state." + state), 10, 20, TEXT, false);
        if (menu.cardState() != 0)
            g.drawString(font, Component.translatable(menu.released() ? "gui.evanscomputermod.card_reader.released"
                    : "gui.evanscomputermod.card_reader.held"), 10, 40, menu.released() ? GOOD : DIM, false);
    }

    @Override
    public void render(GuiGraphics g, int mouseX, int mouseY, float partialTick) {
        super.render(g, mouseX, mouseY, partialTick);
        renderTooltip(g, mouseX, mouseY);
    }
}
//?}
