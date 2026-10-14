package com.example.evanscomputermod.card;

//? if <=1.21.1 {
import com.example.evanscomputermod.phone.PhoneContent;
import com.example.evanscomputermod.phone.PhoneHardware;
import com.example.evanscomputermod.radio.cellular.CellularContent;
import com.example.evanscomputermod.radio.cellular.SimCardItem;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.world.item.ItemStack;

import java.util.LinkedHashMap;
import java.util.Map;
import java.util.UUID;
import java.util.concurrent.ConcurrentHashMap;

/**
 * Clients only ever hold {@link SimSecret#HIDDEN} in place of a SIM's key. Survival
 * inventories are authoritative on the server, but the creative inventory sends whole
 * stacks back ({@code ServerboundSetCreativeModeSlotPacket}), which would replace a
 * card's key with the placeholder. Before the server applies such a stack, this puts
 * the key back: it looks for a card with the same identity (carrier + IMSI) in the
 * player's inventory, the stack being replaced, or one this player held recently
 * (moving a card is "empty the old slot, fill the new one"). A SIM inside an ePhone's
 * SIM tray is handled the same way. Applied by {@code card.mixin.CreativeSlotMixin}.
 *
 * <p>A hidden SIM with no match (it can't be produced honestly) is stored without a
 * key: a written card that answers no challenge.
 */
public final class SimSecretGuard {
    private static final int REMEMBER = 64;
    private static final Map<UUID, Map<String, SimSecret>> RECENT = new ConcurrentHashMap<>();

    private SimSecretGuard() {}

    /** Called before the server applies a creative-inventory stack to {@code slot} (-1: dropped). */
    public static void beforeCreativeSet(ServerPlayer player, int slot, ItemStack incoming) {
        if (incoming == null || incoming.isEmpty()) {
            if (slot >= 0 && slot < player.inventoryMenu.slots.size()) remember(player, player.inventoryMenu.getSlot(slot).getItem());
            return;
        }
        if (!needsRestore(incoming)) return;
        if (slot >= 0 && slot < player.inventoryMenu.slots.size()) remember(player, player.inventoryMenu.getSlot(slot).getItem());
        var inv = player.getInventory();
        for (int i = 0; i < inv.getContainerSize(); i++) remember(player, inv.getItem(i));
        remember(player, player.containerMenu.getCarried());
        restore(player, incoming);
    }

    private static Map<String, SimSecret> recent(ServerPlayer p) {
        return RECENT.computeIfAbsent(p.getUUID(), k -> java.util.Collections.synchronizedMap(new LinkedHashMap<>(16, 0.75f, true) {
            @Override
            protected boolean removeEldestEntry(Map.Entry<String, SimSecret> e) {
                return size() > REMEMBER;
            }
        }));
    }

    private static String idKey(SimIdentity id) {
        return id.carrier() + "|" + id.imsi();
    }

    private static ItemStack simOf(ItemStack stack) {
        if (stack.getItem() instanceof SimCardItem) return stack;
        PhoneHardware h = stack.get(PhoneContent.HARDWARE.get());
        return h != null ? h.sim() : ItemStack.EMPTY;
    }

    private static boolean needsRestore(ItemStack stack) {
        ItemStack sim = simOf(stack);
        if (sim.isEmpty() || SimCardItem.identity(sim) == null) return false;
        SimSecret s = SimCardItem.secret(sim);
        return s == null || !s.known();
    }

    /** Remember the key of a SIM (or a phone's SIM) the player has now. */
    public static void remember(ServerPlayer player, ItemStack stack) {
        if (stack == null || stack.isEmpty()) return;
        ItemStack sim = simOf(stack);
        SimIdentity id = SimCardItem.identity(sim);
        SimSecret s = SimCardItem.secret(sim);
        if (id != null && s != null && s.known()) recent(player).put(idKey(id), s);
    }

    private static void restore(ServerPlayer player, ItemStack incoming) {
        ItemStack sim = simOf(incoming);
        SimIdentity id = SimCardItem.identity(sim);
        SimSecret known = recent(player).get(idKey(id));
        if (sim == incoming) {
            if (known != null) incoming.set(CellularContent.SIM_SECRET.get(), known);
            else incoming.remove(CellularContent.SIM_SECRET.get());
            return;
        }
        ItemStack fixed = sim.copy();
        if (known != null) fixed.set(CellularContent.SIM_SECRET.get(), known);
        else fixed.remove(CellularContent.SIM_SECRET.get());
        incoming.set(PhoneContent.HARDWARE.get(), incoming.get(PhoneContent.HARDWARE.get()).withSim(fixed));
    }
}
//?}
