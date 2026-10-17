package com.example.evanscomputermod.radio.cellular;

//? if <=1.21.1 {
import com.example.evanscomputermod.card.SimIdentity;
import net.minecraft.core.HolderLookup;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.nbt.Tag;
import net.minecraft.server.MinecraftServer;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.saveddata.SavedData;
import net.neoforged.neoforge.server.ServerLifecycleHooks;
import org.jetbrains.annotations.Nullable;

import java.util.Map;
import java.util.UUID;
import java.util.concurrent.ConcurrentHashMap;

/**
 * The SIM card in a Terminal modem's own slot, kept on the server as the real item stack.
 *
 * <p>A modem module's state is custom data on its item (it travels with the item and is
 * synced to clients, in an inventory and in the Terminal's update tag), and a written SIM's
 * key must never reach a client. So the module state holds only the card's public identity
 * and a random slot id ({@code sim: {carrier, imsi, slot}}, see {@link #writeRef}); the SIM
 * stack itself, with its server-only key component and anything else it carries, lives
 * here (server saved data {@code evanscomputermod_modem_sims}) until it is taken out
 * ({@link #take}: the sneak-use of the loose modem item, or a swap in the bay).
 *
 * <p>The slot id is the only way to the card: a copied modem item (creative) shows the
 * identity but its slot is gone once the first copy gives the card back, so a SIM is never
 * duplicated. An entry whose modem is destroyed stays (one item stack). Every method is
 * thread-safe: the {@code sim} peripheral reads the card off the server thread.
 */
public final class ModemSimStore extends SavedData {
    static final String NAME = "evanscomputermod_modem_sims";
    private final Map<String, ItemStack> slots = new ConcurrentHashMap<>();

    private static @Nullable ModemSimStore get() {
        MinecraftServer server = ServerLifecycleHooks.getCurrentServer();
        if (server == null) return null;
        return server.overworld().getDataStorage().computeIfAbsent(
                new SavedData.Factory<>(ModemSimStore::new, ModemSimStore::load, null), NAME);
    }

    /** Keep one card of {@code sim}; returns its new slot id, or null without a server. */
    public static @Nullable String put(ItemStack sim) {
        ModemSimStore s = get();
        if (s == null || sim.isEmpty()) return null;
        String id = UUID.randomUUID().toString();
        s.slots.put(id, sim.copyWithCount(1));
        s.setDirty();
        return id;
    }

    /** The card in slot {@code id} (do not change it), or EMPTY. */
    public static ItemStack peek(@Nullable String id) {
        ModemSimStore s = id == null ? null : get();
        if (s == null) return ItemStack.EMPTY;
        return s.slots.getOrDefault(id, ItemStack.EMPTY);
    }

    /** Take the card out of slot {@code id} (EMPTY if it isn't there). */
    public static ItemStack take(@Nullable String id) {
        ModemSimStore s = id == null ? null : get();
        if (s == null) return ItemStack.EMPTY;
        ItemStack out = s.slots.remove(id);
        if (out == null) return ItemStack.EMPTY;
        s.setDirty();
        return out;
    }

    // ------------------------------------------------------------ the reference in the module state

    /** {@code {carrier, imsi, slot}}: what a modem's (client-visible) state says about its SIM. */
    public static CompoundTag writeRef(SimIdentity id, String slot) {
        CompoundTag t = new CompoundTag();
        t.putString("carrier", id.carrier());
        t.putLong("imsi", id.imsi());
        t.putString("slot", slot);
        return t;
    }

    /** The slot id in a {@link #writeRef} tag, or null. */
    public static @Nullable String slotOf(@Nullable CompoundTag t) {
        return t == null || !t.contains("slot") ? null : t.getString("slot");
    }

    /** Only carrier and IMSI (what a client can know), or null. */
    public static @Nullable SimIdentity identity(@Nullable CompoundTag t) {
        if (t == null || !t.contains("carrier")) return null;
        try {
            return new SimIdentity(t.getString("carrier"), t.getLong("imsi"));
        } catch (IllegalArgumentException e) {
            return null;
        }
    }

    // ------------------------------------------------------------ saved data

    private static ModemSimStore load(CompoundTag tag, HolderLookup.Provider registries) {
        ModemSimStore v = new ModemSimStore();
        CompoundTag k = tag.getCompound("slots");
        for (String id : k.getAllKeys()) {
            ItemStack s = ItemStack.parseOptional(registries, k.getCompound(id));
            if (!s.isEmpty()) v.slots.put(id, s);
        }
        return v;
    }

    @Override
    public CompoundTag save(CompoundTag tag, HolderLookup.Provider registries) {
        CompoundTag k = new CompoundTag();
        slots.forEach((id, s) -> {
            Tag t = s.save(registries);
            k.put(id, t);
        });
        tag.put("slots", k);
        return tag;
    }
}
//?}
