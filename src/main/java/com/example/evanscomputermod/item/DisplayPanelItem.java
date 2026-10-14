package com.example.evanscomputermod.item;

import com.example.evanscomputermod.computer.display.PanelTier;
import net.minecraft.ChatFormatting;
import net.minecraft.network.chat.Component;
import net.minecraft.world.item.ItemStack;
import org.jetbrains.annotations.Nullable;

import java.util.function.Consumer;

/**
 * A display panel: sits in a device's screen slot and sets what the display
 * can do (see {@link PanelTier}). Right-click a Terminal's screen face to
 * install or swap it; sneak-right-click the face with an empty hand to take
 * it out (the computer then runs headless). Crafted each from the tier below.
 */
public class DisplayPanelItem extends TooltipItem {

    private final PanelTier tier;
    private final PanelTier.Tone tone;

    public DisplayPanelItem(Properties properties, PanelTier tier, PanelTier.Tone tone) {
        super(properties);
        this.tier = tier;
        this.tone = tone;
    }

    public PanelTier tier() {
        return tier;
    }

    /** Phosphor colour (only Tier 1 shows it). */
    public PanelTier.Tone tone() {
        return tone;
    }

    /** The tier of a panel stack, or null if it isn't a panel (empty: no panel). */
    @Nullable
    public static PanelTier tierOf(ItemStack stack) {
        return stack.getItem() instanceof DisplayPanelItem p ? p.tier : null;
    }

    public static PanelTier.Tone toneOf(ItemStack stack) {
        return stack.getItem() instanceof DisplayPanelItem p ? p.tone : PanelTier.Tone.GREEN;
    }

    @Override
    protected void addTooltip(ItemStack stack, Consumer<Component> lines) {
        PanelTier.Modes m = tier.modes();
        lines.accept(Component.translatable("item.evanscomputermod.display_panel.tier", tier.tier(),
                Component.translatable("panel_tier.evanscomputermod." + tier.id())).withStyle(ChatFormatting.GOLD));
        lines.accept(Component.translatable("item.evanscomputermod.display_panel.text",
                m.terminal().textCols(), m.terminal().textRows()).withStyle(ChatFormatting.GRAY));
        lines.accept(Component.translatable("item.evanscomputermod.display_panel.gfx",
                m.terminal().gfxWidth(), m.terminal().gfxHeight(), m.refreshHz()).withStyle(ChatFormatting.GRAY));
        lines.accept(Component.translatable("item.evanscomputermod.display_panel.color." + tier.id()).withStyle(ChatFormatting.GRAY));
        lines.accept(Component.translatable("item.evanscomputermod.display_panel.tooltip").withStyle(ChatFormatting.DARK_GRAY));
    }
}
