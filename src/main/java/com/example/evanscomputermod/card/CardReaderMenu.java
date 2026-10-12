package com.example.evanscomputermod.card;

//? if <=1.21.1 {
import net.minecraft.world.entity.player.Inventory;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.inventory.AbstractContainerMenu;
import net.minecraft.world.inventory.ContainerData;
import net.minecraft.world.inventory.SimpleContainerData;
import net.minecraft.world.inventory.Slot;
import net.minecraft.world.item.ItemStack;
import net.neoforged.neoforge.items.ItemStackHandler;
import net.neoforged.neoforge.items.SlotItemHandler;
import org.jetbrains.annotations.Nullable;

/**
 * The Card Reader/Writer screen's menu: the card slot, the player's inventory and the
 * synced state (Auto Eject Sim, card state, released). Button 0 sets Auto Eject Sim to
 * false, button 1 to true (the vanilla menu-button packet, like the enchanting table's
 * buttons).
 */
public class CardReaderMenu extends AbstractContainerMenu {
    public static final int BUTTON_AUTO_EJECT_FALSE = 0, BUTTON_AUTO_EJECT_TRUE = 1;
    public static final int SLOT_X = 148, SLOT_Y = 26;

    private final @Nullable CardReaderBlockEntity be;
    private final ContainerData data;

    /** Client. */
    public CardReaderMenu(int id, Inventory inv) {
        this(id, inv, null, new SimpleContainerData(3));
    }

    /** Server. */
    public CardReaderMenu(int id, Inventory inv, @Nullable CardReaderBlockEntity be, ContainerData data) {
        super(CardContent.CARD_READER_MENU.get(), id);
        this.be = be;
        this.data = data;
        var handler = be != null ? be.slotHandler() : new ItemStackHandler(1);
        addSlot(new SlotItemHandler(handler, 0, SLOT_X, SLOT_Y) {
            @Override
            public boolean mayPlace(ItemStack stack) {
                return CardReaderBlockEntity.isCard(stack);
            }

            @Override
            public int getMaxStackSize() {
                return 1;
            }
        });
        for (int row = 0; row < 3; row++)
            for (int col = 0; col < 9; col++) addSlot(new Slot(inv, col + row * 9 + 9, 8 + col * 18, 84 + row * 18));
        for (int col = 0; col < 9; col++) addSlot(new Slot(inv, col, 8 + col * 18, 142));
        addDataSlots(data);
    }

    public boolean autoEject() {
        return data.get(0) != 0;
    }

    /** 0 no card, 1 blank SIM, 2 written SIM, 3 another card. */
    public int cardState() {
        return data.get(1);
    }

    public boolean released() {
        return data.get(2) != 0;
    }

    @Override
    public boolean clickMenuButton(Player player, int id) {
        if (be == null) return false;
        if (id == BUTTON_AUTO_EJECT_FALSE || id == BUTTON_AUTO_EJECT_TRUE) {
            be.setAutoEject(id == BUTTON_AUTO_EJECT_TRUE);
            return true;
        }
        return false;
    }

    @Override
    public ItemStack quickMoveStack(Player player, int index) {
        Slot s = slots.get(index);
        if (!s.hasItem()) return ItemStack.EMPTY;
        ItemStack stack = s.getItem();
        ItemStack before = stack.copy();
        if (index == 0) {
            if (!moveItemStackTo(stack, 1, slots.size(), true)) return ItemStack.EMPTY;
        } else if (!CardReaderBlockEntity.isCard(stack) || !moveItemStackTo(stack, 0, 1, false)) {
            return ItemStack.EMPTY;
        }
        if (stack.isEmpty()) s.setByPlayer(ItemStack.EMPTY);
        else s.setChanged();
        return before;
    }

    @Override
    public boolean stillValid(Player p) {
        return be == null || !be.isRemoved() && p.distanceToSqr(be.getBlockPos().getCenter()) <= 64;
    }
}
//?}
