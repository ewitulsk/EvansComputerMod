package com.example.evanscomputermod.block;

import com.example.evanscomputermod.computer.display.PanelTier;
import com.example.evanscomputermod.item.DisplayPanelItem;
import net.minecraft.util.StringRepresentable;
import net.minecraft.world.item.ItemStack;
import org.jetbrains.annotations.Nullable;

import java.util.Locale;

/**
 * What a Terminal's screen face shows: its display panel (bezel and tint per
 * tier), or an empty slot. A block-state property ({@link TerminalBlock#PANEL})
 * like the module bay visuals, so it renders as part of the block model; the
 * client also reads it for the Terminal GUI's tier (grid scaling, phosphor
 * tones).
 */
public enum PanelVisual implements StringRepresentable {
    NONE(null, PanelTier.Tone.GREEN),
    PHOSPHOR(PanelTier.PHOSPHOR, PanelTier.Tone.GREEN),
    AMBER(PanelTier.PHOSPHOR, PanelTier.Tone.AMBER),
    COLOR_LCD(PanelTier.COLOR_LCD, PanelTier.Tone.GREEN),
    HD(PanelTier.HD, PanelTier.Tone.GREEN);

    @Nullable
    private final PanelTier tier;
    private final PanelTier.Tone tone;

    PanelVisual(@Nullable PanelTier tier, PanelTier.Tone tone) {
        this.tier = tier;
        this.tone = tone;
    }

    /** Null for {@link #NONE}. */
    @Nullable
    public PanelTier tier() {
        return tier;
    }

    public PanelTier.Tone tone() {
        return tone;
    }

    public static PanelVisual of(ItemStack panel) {
        PanelTier t = DisplayPanelItem.tierOf(panel);
        if (t == null) return NONE;
        return switch (t) {
            case PHOSPHOR -> DisplayPanelItem.toneOf(panel) == PanelTier.Tone.AMBER ? AMBER : PHOSPHOR;
            case COLOR_LCD -> COLOR_LCD;
            case HD -> HD;
        };
    }

    @Override
    public String getSerializedName() {
        return name().toLowerCase(Locale.ROOT);
    }
}
