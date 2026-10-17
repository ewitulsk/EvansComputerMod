package com.example.evanscomputermod.card.printer;

//? if <=1.21.1 {
import com.example.evanscomputermod.card.CardContent;
import com.example.evanscomputermod.card.CardKind;
import net.minecraft.world.entity.player.Inventory;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.inventory.AbstractContainerMenu;
import net.minecraft.world.inventory.ContainerData;
import net.minecraft.world.inventory.SimpleContainerData;
import net.minecraft.world.inventory.Slot;
import net.minecraft.world.item.ItemStack;
import net.neoforged.neoforge.items.IItemHandler;
import net.neoforged.neoforge.items.ItemStackHandler;
import net.neoforged.neoforge.items.SlotItemHandler;
import org.jetbrains.annotations.Nullable;

/**
 * The Card Printer screen's menu: the card in, the card out, sixteen dye slots, the
 * player's inventory, and the synced state (Auto Eject, progress, the cards' kinds).
 * Button 0 / 1 set Auto Eject false / true.
 */
public class CardPrinterMenu extends AbstractContainerMenu {
    public static final int BUTTON_AUTO_EJECT_FALSE = 0, BUTTON_AUTO_EJECT_TRUE = 1;
    public static final int IN_X = 26, IN_Y = 20, OUT_X = 26, OUT_Y = 50, DYE_X = 62, DYE_Y = 17;
    /** The player's inventory starts below the printer's own rows. */
    public static final int INV_Y = 108;

    private final @Nullable CardPrinterBlockEntity be;
    private final ContainerData data;

    public CardPrinterMenu(int id, Inventory inv) {
        this(id, inv, null, new SimpleContainerData(4));
    }

    public CardPrinterMenu(int id, Inventory inv, @Nullable CardPrinterBlockEntity be, ContainerData data) {
        super(CardContent.CARD_PRINTER_MENU.get(), id);
        this.be = be;
        this.data = data;
        IItemHandler h = be != null ? be.slotHandler() : new ItemStackHandler(CardPrinterBlockEntity.SLOTS);
        addSlot(new SlotItemHandler(h, CardPrinterBlockEntity.SLOT_IN, IN_X, IN_Y) {
            @Override
            public boolean mayPlace(ItemStack stack) {
                return h.isItemValid(CardPrinterBlockEntity.SLOT_IN, stack) && (be == null || !be.printing());
            }

            @Override
            public boolean mayPickup(Player p) {
                return be == null || !be.printing();
            }

            @Override
            public int getMaxStackSize() {
                return 1;
            }
        });
        addSlot(new SlotItemHandler(h, CardPrinterBlockEntity.SLOT_OUT, OUT_X, OUT_Y) {
            @Override
            public boolean mayPlace(ItemStack stack) {
                return false;
            }
        });
        for (int i = 0; i < 16; i++) {
            int slot = CardPrinterBlockEntity.SLOT_DYE0 + i;
            addSlot(new SlotItemHandler(h, slot, DYE_X + (i % 8) * 18, DYE_Y + (i / 8) * 18) {
                @Override
                public boolean mayPlace(ItemStack stack) {
                    return h.isItemValid(slot, stack);
                }
            });
        }
        for (int row = 0; row < 3; row++)
            for (int col = 0; col < 9; col++) addSlot(new Slot(inv, col + row * 9 + 9, 8 + col * 18, INV_Y + row * 18));
        for (int col = 0; col < 9; col++) addSlot(new Slot(inv, col, 8 + col * 18, INV_Y + 58));
        addDataSlots(data);
    }

    public boolean autoEject() {
        return data.get(0) != 0;
    }

    /** -1 idle, else 0..PRINT_TICKS. */
    public int progress() {
        return data.get(1);
    }

    public @Nullable CardKind kindIn() {
        int k = data.get(2) - 1;
        return k < 0 || k >= CardKind.values().length ? null : CardKind.values()[k];
    }

    public @Nullable CardKind kindOut() {
        int k = data.get(3) - 1;
        return k < 0 || k >= CardKind.values().length ? null : CardKind.values()[k];
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
        Slot slot = slots.get(index);
        if (!slot.hasItem()) return ItemStack.EMPTY;
        ItemStack stack = slot.getItem(), copy = stack.copy();
        int own = CardPrinterBlockEntity.SLOTS;
        if (index < own) {
            if (!moveItemStackTo(stack, own, own + 36, true)) return ItemStack.EMPTY;
        } else {
            boolean moved = false;
            for (int i = 0; i < own && !moved; i++) if (slots.get(i).mayPlace(stack)) moved = moveItemStackTo(stack, i, i + 1, false);
            if (!moved) return ItemStack.EMPTY;
        }
        if (stack.isEmpty()) slot.setByPlayer(ItemStack.EMPTY);
        else slot.setChanged();
        return copy;
    }

    @Override
    public boolean stillValid(Player player) {
        return be == null || be.getLevel() != null && be.getLevel().getBlockEntity(be.getBlockPos()) == be
                && player.distanceToSqr(be.getBlockPos().getCenter()) <= 64;
    }
}
//?}
