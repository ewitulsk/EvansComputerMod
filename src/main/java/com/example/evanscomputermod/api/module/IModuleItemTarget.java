package com.example.evanscomputermod.api.module;

import net.minecraft.network.chat.Component;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.item.ItemStack;

/**
 * A module that takes items while it sits in a bay: right-clicking the
 * module's bay slot with an item it accepts hands the item to the module
 * instead of trying to install it (a cellular modem takes a SIM card).
 */
public interface IModuleItemTarget {

    /** Whether right-clicking this module's slot with {@code stack} is meant for it. */
    boolean acceptsItem(ItemStack stack);

    /**
     * Take what it needs from {@code stack} (server side; shrink it unless the player
     * is in creative), giving anything it hands back to {@code player}. Returns the
     * action-bar message.
     */
    Component useItem(ItemStack stack, Player player);
}
