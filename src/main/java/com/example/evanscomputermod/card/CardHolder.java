package com.example.evanscomputermod.card;

import net.minecraft.world.item.ItemStack;

/**
 * Something with one contact card slot a {@link CardReaderPeripheral} works over: the
 * Card Reader/Writer block and the card reader bay module. The held / released life cycle
 * is the holder's (see {@link CardReaderBlockEntity}).
 */
public interface CardHolder {
    /** The card in the slot (EMPTY with none). Changes made to it in place are the card's. */
    ItemStack card();

    default boolean hasCard() {
        return !card().isEmpty();
    }

    /** The computer is done with the card; true if Auto Eject pushed it out. */
    boolean release();

    /** The card was changed in place (written): save it. */
    void cardChanged();

    boolean released();

    boolean autoEject();
}
