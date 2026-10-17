package com.example.evanscomputermod.computer.peripheral.generic;

//? if <=1.21.1 {
import com.example.evanscomputermod.api.peripheral.IComputerAccess;
import com.example.evanscomputermod.api.peripheral.IPeripheral;
import com.example.evanscomputermod.api.peripheral.PeripheralException;
import com.example.evanscomputermod.api.peripheral.PeripheralMethod;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.Level;
import net.neoforged.neoforge.capabilities.Capabilities;
import net.neoforged.neoforge.items.IItemHandler;
import net.neoforged.neoforge.items.ItemHandlerHelper;
import org.jetbrains.annotations.Nullable;

import java.util.LinkedHashMap;
import java.util.Map;
import java.util.TreeMap;

/**
 * The {@code inventory} methods of a {@link GenericPeripheral} (any block with
 * NeoForge's item capability), as ComputerCraft has them. Slots are numbered
 * from 1. Items are described by registry name and count; details add
 * {@code displayName}, {@code maxCount}, damage and tags (CC's field names).
 */
public final class InventoryMethods implements IPeripheral {
    private final Level level;
    private final BlockPos pos;
    private final Direction side;

    InventoryMethods(Level level, BlockPos pos, Direction side) {
        this.level = level;
        this.pos = pos;
        this.side = side;
    }

    // Only the annotated methods are used (through GenericPeripheral); these
    // make the class registrable with PeripheralTypes for the catalogue.
    @Override public String getType() { return GenericPeripheral.INVENTORY; }
    @Override public java.util.Set<String> getMethodNames() { return java.util.Set.of(); }
    @Override public Object callMethod(IComputerAccess c, String m, Object[] a) throws PeripheralException {
        throw new PeripheralException("no method " + m);
    }

    IItemHandler handler() throws PeripheralException {
        IItemHandler h = level.getCapability(Capabilities.ItemHandler.BLOCK, pos, side);
        if (h == null) throw new PeripheralException("the inventory is gone");
        return h;
    }

    private static int slot(IItemHandler h, int slot) throws PeripheralException {
        if (slot < 1 || slot > h.getSlots()) throw new PeripheralException("Slot out of range (1-" + h.getSlots() + ")");
        return slot - 1;
    }

    private static Map<String, Object> basic(ItemStack s) {
        Map<String, Object> m = new LinkedHashMap<>();
        m.put("name", BuiltInRegistries.ITEM.getKey(s.getItem()).toString());
        m.put("count", s.getCount());
        return m;
    }

    @PeripheralMethod(description = "Number of slots")
    public int size() throws PeripheralException {
        return handler().getSlots();
    }

    @PeripheralMethod(description = "The items in the inventory: {slot = {name, count}}, empty slots left out")
    public Map<Integer, Object> list() throws PeripheralException {
        IItemHandler h = handler();
        Map<Integer, Object> out = new TreeMap<>();
        for (int i = 0; i < h.getSlots(); i++) {
            ItemStack s = h.getStackInSlot(i);
            if (!s.isEmpty()) out.put(i + 1, basic(s));
        }
        return out;
    }

    @PeripheralMethod(description = "The item in a slot in detail (name, count, displayName, maxCount, damage, tags), or nil")
    @Nullable
    public Map<String, Object> getItemDetail(int slot) throws PeripheralException {
        IItemHandler h = handler();
        ItemStack s = h.getStackInSlot(slot(h, slot));
        if (s.isEmpty()) return null;
        Map<String, Object> m = basic(s);
        m.put("displayName", s.getHoverName().getString());
        m.put("maxCount", s.getMaxStackSize());
        if (s.isDamageableItem()) {
            m.put("damage", s.getDamageValue());
            m.put("maxDamage", s.getMaxDamage());
        }
        Map<String, Object> tags = new TreeMap<>();
        s.getTags().forEach(t -> tags.put(t.location().toString(), true));
        m.put("tags", tags);
        return m;
    }

    @PeripheralMethod(description = "How many items a slot can hold")
    public int getItemLimit(int slot) throws PeripheralException {
        IItemHandler h = handler();
        return h.getSlotLimit(slot(h, slot));
    }

    @PeripheralMethod(description = "Move items from a slot into the inventory attached as to_name (to its slot to_slot, or anywhere): the number moved")
    public int pushItems(IComputerAccess computer, String toName, int fromSlot, @Nullable Integer limit, @Nullable Integer toSlot)
            throws PeripheralException {
        InventoryMethods to = target(computer, toName);
        IItemHandler from = handler(), dest = to.handler();
        return move(from, slot(from, fromSlot), dest, toSlot == null ? -1 : slot(dest, toSlot), limit);
    }

    @PeripheralMethod(description = "Move items from a slot of the inventory attached as from_name into this one: the number moved")
    public int pullItems(IComputerAccess computer, String fromName, int fromSlot, @Nullable Integer limit, @Nullable Integer toSlot)
            throws PeripheralException {
        InventoryMethods src = target(computer, fromName);
        IItemHandler from = src.handler(), dest = handler();
        return move(from, slot(from, fromSlot), dest, toSlot == null ? -1 : slot(dest, toSlot), limit);
    }

    private static InventoryMethods target(IComputerAccess computer, String name) throws PeripheralException {
        InventoryMethods inv = GenericPeripheral.partner(computer, name, "an inventory").inventory();
        if (inv == null) throw new PeripheralException("Target '" + name + "' is not an inventory");
        return inv;
    }

    /** Move up to {@code limit} items (a whole stack if null) from a slot into a slot (or anywhere: -1). */
    static int move(IItemHandler from, int fromSlot, IItemHandler to, int toSlot, @Nullable Integer limit) throws PeripheralException {
        int max = limit == null ? Integer.MAX_VALUE : limit;
        if (max < 0) throw new PeripheralException("Limit must be 0 or more");
        if (max == 0) return 0;
        ItemStack offered = from.extractItem(fromSlot, max, true);
        if (offered.isEmpty()) return 0;
        ItemStack left = toSlot < 0 ? ItemHandlerHelper.insertItem(to, offered, true) : to.insertItem(toSlot, offered, true);
        int fits = offered.getCount() - left.getCount();
        if (fits <= 0) return 0;
        ItemStack taken = from.extractItem(fromSlot, fits, false);
        ItemStack back = toSlot < 0 ? ItemHandlerHelper.insertItem(to, taken, false) : to.insertItem(toSlot, taken, false);
        if (!back.isEmpty()) from.insertItem(fromSlot, back, false);
        return taken.getCount() - back.getCount();
    }
}
//?}
