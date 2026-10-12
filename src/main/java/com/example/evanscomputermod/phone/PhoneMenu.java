package com.example.evanscomputermod.phone;

//? if <=1.21.1 {
import com.example.evanscomputermod.api.module.IComputerModuleItem;
import com.example.evanscomputermod.item.DisplayPanelItem;
import com.example.evanscomputermod.radio.cellular.SimCardItem;
import net.minecraft.network.RegistryFriendlyByteBuf;
import net.minecraft.world.Container;
import net.minecraft.world.InteractionHand;
import net.minecraft.world.SimpleContainer;
import net.minecraft.world.entity.player.Inventory;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.inventory.AbstractContainerMenu;
import net.minecraft.world.inventory.Slot;
import net.minecraft.world.item.ItemStack;
import org.jetbrains.annotations.Nullable;

import java.util.UUID;

/**
 * The phone's slot screen (sneak + right-click with the phone in hand): the
 * display panel, two module slots and the SIM tray, plus the player's
 * inventory. Changes apply live: a new panel resizes the console, modules
 * start and stop, the SIM is read by the modems every tick.
 */
public class PhoneMenu extends AbstractContainerMenu {
    public static final int PANEL = 0, MODULE_1 = 1, MODULE_2 = 2, SIM = 3, SLOTS = 4;
    /** Slot positions in the 176 x 186 phone screen. */
    public static final int[][] POS = {{80, 14}, {58, 42}, {102, 42}, {80, 70}};

    private final Player player;
    private final InteractionHand hand;
    @Nullable private final UUID phone;
    private final SimpleContainer slots = new SimpleContainer(SLOTS) {
        @Override
        public int getMaxStackSize() {
            return 1;
        }

        @Override
        public void setChanged() {
            super.setChanged();
            PhoneMenu.this.onSlotsChanged();
        }
    };
    private boolean loading;

    /** Client. */
    public PhoneMenu(int containerId, Inventory inv, RegistryFriendlyByteBuf buf) {
        this(containerId, inv, buf.readBoolean() ? InteractionHand.MAIN_HAND : InteractionHand.OFF_HAND, null);
    }

    /** Server. */
    public PhoneMenu(int containerId, Inventory inv, InteractionHand hand, @Nullable UUID phone) {
        super(PhoneContent.PHONE_MENU.get(), containerId);
        this.player = inv.player;
        this.hand = hand;
        this.phone = phone;
        if (phone != null) {
            loading = true;
            PhoneHardware h = PhoneHardware.of(player.getItemInHand(hand));
            for (int i = 0; i < SLOTS; i++) slots.setItem(i, h.slot(i).copy());
            loading = false;
        }
        for (int i = 0; i < SLOTS; i++) addSlot(new PartSlot(slots, i, POS[i][0], POS[i][1]));
        int lockedSlot = hand == InteractionHand.MAIN_HAND ? inv.selected : -1;
        for (int row = 0; row < 3; row++)
            for (int col = 0; col < 9; col++)
                addSlot(new Slot(inv, col + row * 9 + 9, 8 + col * 18, 104 + row * 18));
        for (int col = 0; col < 9; col++) {
            int index = col;
            addSlot(new Slot(inv, col, 8 + col * 18, 162) {
                @Override
                public boolean mayPickup(Player p) {
                    return index != lockedSlot && super.mayPickup(p);
                }
            });
        }
    }

    public InteractionHand hand() {
        return hand;
    }

    /** Whether {@code stack} fits part slot {@code slot}. */
    public static boolean fits(int slot, ItemStack stack) {
        return switch (slot) {
            case PANEL -> stack.getItem() instanceof DisplayPanelItem;
            case MODULE_1, MODULE_2 -> stack.getItem() instanceof IComputerModuleItem;
            case SIM -> stack.getItem() instanceof SimCardItem;
            default -> false;
        };
    }

    private static final class PartSlot extends Slot {
        PartSlot(Container c, int index, int x, int y) {
            super(c, index, x, y);
        }

        @Override
        public boolean mayPlace(ItemStack stack) {
            return fits(getContainerSlot(), stack);
        }

        @Override
        public int getMaxStackSize() {
            return 1;
        }
    }

    /** Server: write the slots back to the phone in hand and apply them live. */
    private void onSlotsChanged() {
        if (loading || phone == null || player.level().isClientSide()) return;
        ItemStack stack = player.getItemInHand(hand);
        if (!phone.equals(PhoneItemData.id(stack))) return;
        PhoneHardware h = new PhoneHardware(slots.getItem(0), slots.getItem(1), slots.getItem(2), slots.getItem(3));
        PhoneHost host = player.getServer() != null ? PhoneServer.find(player.getServer(), phone) : null;
        if (host != null && host.isAttached() && player.getUUID().equals(host.holderId())) host.setHardware(h);
        else stack.set(PhoneContent.HARDWARE.get(), h);
    }

    @Override
    public ItemStack quickMoveStack(Player p, int index) {
        Slot slot = getSlot(index);
        if (!slot.hasItem()) return ItemStack.EMPTY;
        ItemStack stack = slot.getItem();
        ItemStack copy = stack.copy();
        if (index < SLOTS) {
            if (!moveItemStackTo(stack, SLOTS, SLOTS + 36, true)) return ItemStack.EMPTY;
        } else {
            boolean moved = false;
            for (int i = 0; i < SLOTS && !moved; i++) {
                if (fits(i, stack) && !getSlot(i).hasItem()) moved = moveItemStackTo(stack, i, i + 1, false);
            }
            if (!moved) return ItemStack.EMPTY;
        }
        if (stack.isEmpty()) slot.setByPlayer(ItemStack.EMPTY);
        else slot.setChanged();
        return copy;
    }

    @Override
    public boolean stillValid(Player p) {
        if (phone == null) return true;
        return phone.equals(PhoneItemData.id(p.getItemInHand(hand)));
    }
}
//?}
