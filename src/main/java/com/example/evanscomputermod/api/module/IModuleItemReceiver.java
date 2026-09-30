package com.example.evanscomputermod.api.module;

import net.minecraft.world.item.ItemStack;

/**
 * An installed module that holds an item of its own (the Storage Module holds
 * a Storage Cell). Right-clicking its bay slot with an accepted item puts one
 * in; sneak-right-clicking the slot with an empty hand takes that item out
 * before the module itself comes out.
 *
 * <p>Items it accepts must also be registered with
 * {@code ModuleInteraction.registerInsertable} so the computer treats the click
 * as a bay action.
 */
public interface IModuleItemReceiver {

    boolean accepts(ItemStack stack);

    /** Take one of {@code stack} (the caller shrinks it). False if full or not accepted. */
    boolean insert(ItemStack stack);

    /** Remove the held item; EMPTY if there is none. */
    ItemStack takeOut();
}
