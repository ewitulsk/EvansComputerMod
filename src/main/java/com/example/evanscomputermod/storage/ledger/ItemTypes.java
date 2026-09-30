package com.example.evanscomputermod.storage.ledger;

//? if <=1.21.1 {

import com.example.evanscomputermod.storage.core.StorageException;
import it.unimi.dsi.fastutil.ints.IntSet;
import it.unimi.dsi.fastutil.objects.Object2IntOpenCustomHashMap;
import net.minecraft.core.HolderLookup;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.nbt.ListTag;
import net.minecraft.nbt.NbtIo;
import net.minecraft.nbt.Tag;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.ItemStackLinkedSet;
import org.jetbrains.annotations.Nullable;

import java.io.ByteArrayOutputStream;
import java.io.DataOutputStream;
import java.io.IOException;
import java.util.HashMap;
import java.util.Map;

/**
 * Interned item types: each distinct item + components combination gets a
 * stable integer id, saved with the ledger. Programs see ids as short keys
 * ({@code "k3f"}) and never the stacks themselves, so they can't forge one.
 *
 * <p>Thread-safe: the server thread interns, program threads (item overlays)
 * look up.
 */
public final class ItemTypes {

    private final Map<Integer, ItemStack> byId = new HashMap<>();
    private final Object2IntOpenCustomHashMap<ItemStack> ids =
            new Object2IntOpenCustomHashMap<>(ItemStackLinkedSet.TYPE_AND_TAG);
    /** Saved types whose item no longer exists (mod removed); kept so they come back if it returns. */
    private final Map<Integer, CompoundTag> unresolved = new HashMap<>();
    private int nextId = 1;

    public ItemTypes() {
        ids.defaultReturnValue(0);
    }

    public static String keyOf(int id) {
        return "k" + Integer.toString(id, 36);
    }

    /** Parse {@code "k3f"}; 0 if malformed. */
    public static int parseKey(@Nullable String key) {
        if (key == null || key.length() < 2 || key.charAt(0) != 'k') return 0;
        try {
            int v = Integer.parseInt(key.substring(1), 36);
            return v > 0 ? v : 0;
        } catch (NumberFormatException e) {
            return 0;
        }
    }

    /** Id of an already-known type, 0 if never interned. */
    public synchronized int find(ItemStack stack) {
        if (stack.isEmpty()) return 0;
        return ids.getInt(stack);
    }

    /**
     * Id for {@code stack}'s type, interning it if new.
     *
     * @param maxBytes largest allowed encoded size of a new type (0 = no limit)
     */
    public synchronized int intern(ItemStack stack, HolderLookup.Provider registries, int maxBytes) throws StorageException {
        if (stack.isEmpty()) throw new StorageException("empty stack");
        int id = ids.getInt(stack);
        if (id != 0) return id;
        ItemStack proto = stack.copyWithCount(1);
        if (maxBytes > 0 && encodedSize(proto, registries) > maxBytes) {
            throw new StorageException("item data is too large to store (over " + maxBytes + " bytes)");
        }
        id = nextId++;
        byId.put(id, proto);
        ids.put(proto, id);
        return id;
    }

    /** Prototype (count 1) of a type. Callers must copy before handing it out. */
    @Nullable
    public synchronized ItemStack get(int id) {
        return byId.get(id);
    }

    public synchronized int size() {
        return byId.size();
    }

    /** Drop types nothing refers to any more. */
    public synchronized void retain(IntSet used) {
        unresolved.keySet().removeIf(id -> !used.contains((int) id));
        var it = byId.entrySet().iterator();
        while (it.hasNext()) {
            var e = it.next();
            if (!used.contains((int) e.getKey())) {
                ids.removeInt(e.getValue());
                it.remove();
            }
        }
    }

    static int encodedSize(ItemStack proto, HolderLookup.Provider registries) {
        Tag tag = proto.save(registries);
        ByteArrayOutputStream bytes = new ByteArrayOutputStream();
        try (DataOutputStream out = new DataOutputStream(bytes)) {
            CompoundTag wrap = new CompoundTag();
            wrap.put("s", tag);
            NbtIo.write(wrap, out);
        } catch (IOException e) {
            return Integer.MAX_VALUE;
        }
        return bytes.size();
    }

    // ------------------------------------------------------------ persistence

    synchronized ListTag save(HolderLookup.Provider registries) {
        ListTag list = new ListTag();
        for (var e : byId.entrySet()) {
            CompoundTag t = new CompoundTag();
            t.putInt("id", e.getKey());
            t.put("stack", e.getValue().save(registries));
            list.add(t);
        }
        for (var e : unresolved.entrySet()) {
            CompoundTag t = new CompoundTag();
            t.putInt("id", e.getKey());
            t.put("stack", e.getValue().copy());
            list.add(t);
        }
        return list;
    }

    synchronized void load(ListTag list, int savedNext, HolderLookup.Provider registries) {
        byId.clear();
        ids.clear();
        unresolved.clear();
        int max = 0;
        for (int i = 0; i < list.size(); i++) {
            CompoundTag t = list.getCompound(i);
            int id = t.getInt("id");
            ItemStack stack = ItemStack.parse(registries, t.getCompound("stack")).orElse(ItemStack.EMPTY);
            if (id <= 0) continue;
            max = Math.max(max, id);
            if (stack.isEmpty()) { // an item from a removed mod
                unresolved.put(id, t.getCompound("stack").copy());
                continue;
            }
            byId.put(id, stack);
            ids.put(stack, id);
        }
        nextId = Math.max(savedNext, max + 1);
    }

    synchronized int nextId() {
        return nextId;
    }
}
//?}
