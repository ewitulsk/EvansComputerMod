package com.example.evanscomputermod.module;

import com.mojang.serialization.Codec;
import com.mojang.serialization.codecs.RecordCodecBuilder;
import net.minecraft.network.RegistryFriendlyByteBuf;
import net.minecraft.network.codec.StreamCodec;
import net.minecraft.world.item.ItemStack;

/**
 * The display panel in a terminal's screen slot, as saved in the block entity
 * and carried on the terminal item when the block is broken
 * ({@code evanscomputermod:installed_panel}, next to {@code installed_modules}).
 * An empty stack means no panel: the computer runs headless. A terminal item
 * without this component (crafted before panels existed) places a Tier 2
 * terminal, like old worlds load.
 */
public record InstalledPanel(ItemStack panel) {

    public static final InstalledPanel NONE = new InstalledPanel(ItemStack.EMPTY);

    public static final Codec<InstalledPanel> CODEC = RecordCodecBuilder.create(i -> i.group(
            ItemStack.OPTIONAL_CODEC.fieldOf("panel").forGetter(InstalledPanel::panel)
    ).apply(i, InstalledPanel::new));

    public static final StreamCodec<RegistryFriendlyByteBuf, InstalledPanel> STREAM_CODEC =
            ItemStack.OPTIONAL_STREAM_CODEC.map(InstalledPanel::new, InstalledPanel::panel);

    public InstalledPanel {
        panel = panel.copyWithCount(panel.isEmpty() ? 0 : 1);
    }

    public boolean isEmpty() {
        return panel.isEmpty();
    }

    // ItemStack has identity equality; data components need value equality.
    @Override
    public boolean equals(Object o) {
        return this == o || (o instanceof InstalledPanel other && ItemStack.matches(panel, other.panel));
    }

    @Override
    public int hashCode() {
        return ItemStack.hashItemAndComponents(panel);
    }
}
