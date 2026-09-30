package com.example.evanscomputermod.storage.ledger;

//? if <=1.21.1 {

import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.storage.StorageConfig;
import com.example.evanscomputermod.storage.StorageEvents;
import com.example.evanscomputermod.storage.core.CellContents;
import com.example.evanscomputermod.storage.core.CellTier;
import com.example.evanscomputermod.storage.core.StorageException;
import com.example.evanscomputermod.storage.core.TokenBook;
import it.unimi.dsi.fastutil.ints.IntOpenHashSet;
import net.minecraft.core.HolderLookup;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.nbt.ListTag;
import net.minecraft.nbt.Tag;
import net.minecraft.server.MinecraftServer;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.saveddata.SavedData;
import org.jetbrains.annotations.Nullable;

import java.util.Collections;
import java.util.HashMap;
import java.util.HashSet;
import java.util.LinkedHashMap;
import java.util.Map;
import java.util.Set;
import java.util.UUID;

/**
 * The world's item storage ledger: the only authority on what every Storage
 * Cell holds, which tokens are unspent, and what is in lost &amp; found. Saved
 * with the overworld, so it is shared by all dimensions.
 *
 * <p>All mutation happens on the server thread (peripheral calls run there,
 * one at a time), and each operation changes the ledger fully or not at all.
 * That serialisation is the whole defence against double spends: a program
 * can copy any bytes it sees, but only the ledger says who owns what.
 */
public final class StorageLedger extends SavedData implements TokenBook.Cells {

    public static final String NAME = EvansComputerMod.MODID + "_storage";

    private static final class CellRecord {
        CellContents contents;
        long lastSeen;

        CellRecord(CellContents contents, long lastSeen) {
            this.contents = contents;
            this.lastSeen = lastSeen;
        }
    }

    private final Map<UUID, CellRecord> cells = new LinkedHashMap<>();
    private final ItemTypes types = new ItemTypes();
    private final TokenBook tokens = new TokenBook();

    // Not saved.
    private final Map<UUID, MountRef> mounts = new HashMap<>();
    private Set<UUID> pendingChanged = new HashSet<>();
    private Set<UUID> readyChanged = new HashSet<>();
    private MinecraftServer server;

    @Nullable
    private static volatile StorageLedger current;

    private StorageLedger() {
    }

    private static SavedData.Factory<StorageLedger> factory() {
        return new SavedData.Factory<>(StorageLedger::new, StorageLedger::load, null);
    }

    /** The ledger of {@code server}'s world. Server thread. */
    public static StorageLedger get(MinecraftServer server) {
        StorageLedger l = current;
        if (l != null && l.server == server) return l;
        l = server.overworld().getDataStorage().computeIfAbsent(factory(), NAME);
        l.server = server;
        current = l;
        return l;
    }

    /** The ledger if it has been loaded; safe from any thread. */
    @Nullable
    public static StorageLedger peek() {
        return current;
    }

    /** Forget the cached instance (server stopping). */
    public static void release() {
        current = null;
    }

    public MinecraftServer server() {
        return server;
    }

    public HolderLookup.Provider registries() {
        return server.registryAccess();
    }

    public long now() {
        return server.overworld().getGameTime();
    }

    public ItemTypes types() {
        return types;
    }

    public TokenBook tokens() {
        return tokens;
    }

    // ------------------------------------------------------------ cells

    /** Contents of an existing cell, or null. */
    @Override
    @Nullable
    public CellContents get(UUID cell) {
        CellRecord r = cells.get(cell);
        return r == null ? null : r.contents;
    }

    public boolean exists(UUID cell) {
        return cells.containsKey(cell);
    }

    /** Contents of a cell, creating an empty record if it's new. */
    public CellContents ensureCell(UUID cell, CellTier tier) {
        CellRecord r = cells.get(cell);
        if (r == null) {
            r = new CellRecord(new CellContents(tier), now());
            cells.put(cell, r);
            setDirty();
        } else if (r.contents.tier() != tier) {
            CellContents c = new CellContents(tier);
            for (var e : r.contents.view().entrySet()) c.putUnchecked(e.getKey(), e.getValue());
            r.contents = c;
            setDirty();
        }
        return r.contents;
    }

    /** The cell was destroyed: its contents are gone. */
    public void voidCell(UUID cell) {
        if (mounts.containsKey(cell) && mounts.get(cell).isAlive()) return; // a live copy still uses it
        if (cells.remove(cell) != null) {
            EvansComputerMod.LOGGER.debug("Storage cell {} destroyed; contents voided", cell);
            setDirty();
        }
    }

    public void touch(UUID cell) {
        CellRecord r = cells.get(cell);
        if (r != null) {
            r.lastSeen = now();
            setDirty();
        }
    }

    @Override
    public void changed(UUID cell) {
        pendingChanged.add(cell);
        setDirty();
    }

    /** Whether a cell changed in the last tick (read by devices to post {@code storage_changed}). */
    public boolean changedLastTick(UUID cell) {
        return readyChanged.contains(cell);
    }

    public boolean anyChangedLastTick() {
        return !readyChanged.isEmpty();
    }

    // ------------------------------------------------------------ mounts

    /** Mount a cell; false if a live copy is mounted elsewhere. */
    public boolean tryMount(UUID cell, MountRef ref) {
        MountRef old = mounts.get(cell);
        if (old != null && old != ref && old.isAlive()) return false;
        mounts.put(cell, ref);
        touch(cell);
        return true;
    }

    public void unmount(UUID cell, MountRef ref) {
        if (mounts.get(cell) == ref) {
            mounts.remove(cell);
            touch(cell);
        }
    }

    @Nullable
    public MountRef mountOf(UUID cell) {
        return mounts.get(cell);
    }

    // ------------------------------------------------------------ item types

    public int intern(ItemStack stack) throws StorageException {
        return types.intern(stack, registries(), StorageConfig.maxTypeBytes());
    }

    /** A fresh copy of a type's stack with {@code count}, or EMPTY for an unknown id. */
    public ItemStack stackOf(int key, int count) {
        ItemStack proto = types.get(key);
        return proto == null ? ItemStack.EMPTY : proto.copyWithCount(count);
    }

    // ------------------------------------------------------------ ticking (server thread)

    /** End of a server tick: publish this tick's changes, expire tokens, collect lost cells. */
    public void endTick(long tick) {
        Set<UUID> t = readyChanged;
        readyChanged = pendingChanged;
        pendingChanged = t;
        pendingChanged.clear();

        if (tick % 20 == 0 && !tokens.all().isEmpty()) {
            var expired = tokens.expire(this, now());
            if (!expired.isEmpty()) {
                setDirty();
                for (TokenBook.Expired e : expired) StorageEvents.tokenExpired(this, e);
            }
        }
        if (tick % 6000 == 0) {
            long n = now();
            for (UUID cell : mounts.keySet()) {
                CellRecord r = cells.get(cell);
                if (r != null) r.lastSeen = n;
            }
            long gc = StorageConfig.cellGcTicks();
            if (gc > 0) {
                var it = cells.entrySet().iterator();
                while (it.hasNext()) {
                    var e = it.next();
                    if (!mounts.containsKey(e.getKey()) && n - e.getValue().lastSeen > gc) {
                        EvansComputerMod.LOGGER.info("Storage cell {} unseen for {} ticks; contents removed", e.getKey(), gc);
                        it.remove();
                    }
                }
            }
            setDirty();
        }
    }

    // ------------------------------------------------------------ persistence

    @Override
    public CompoundTag save(CompoundTag tag, HolderLookup.Provider registries) {
        IntOpenHashSet used = new IntOpenHashSet();
        for (CellRecord r : cells.values()) used.addAll(r.contents.view().keySet());
        tokens.forEachKey(used::add);
        types.retain(used);

        tag.putInt("version", 1);
        ListTag cl = new ListTag();
        for (var e : cells.entrySet()) {
            CompoundTag c = new CompoundTag();
            c.putUUID("id", e.getKey());
            c.putString("tier", e.getValue().contents.tier().id());
            c.putLong("last_seen", e.getValue().lastSeen);
            ListTag items = new ListTag();
            for (var it : e.getValue().contents.view().entrySet()) {
                CompoundTag i = new CompoundTag();
                i.putInt("k", it.getKey());
                i.putLong("n", it.getValue());
                items.add(i);
            }
            c.put("items", items);
            cl.add(c);
        }
        tag.put("cells", cl);
        tag.put("types", types.save(registries));
        tag.putInt("next_type", types.nextId());

        ListTag tl = new ListTag();
        for (TokenBook.Token t : tokens.all()) {
            CompoundTag c = new CompoundTag();
            c.putString("id", t.id());
            c.putInt("k", t.key());
            c.putLong("n", t.count());
            c.putUUID("origin", t.origin());
            c.putUUID("issuer", t.issuer());
            if (t.boundTo() != null) c.putUUID("bound", t.boundTo());
            c.putLong("expires", t.expiresAt());
            tl.add(c);
        }
        tag.put("tokens", tl);

        ListTag ll = new ListTag();
        for (var e : tokens.lostAll().entrySet()) {
            for (var it : e.getValue().entrySet()) {
                CompoundTag c = new CompoundTag();
                c.putUUID("issuer", e.getKey());
                c.putInt("k", it.getKey());
                c.putLong("n", it.getValue());
                ll.add(c);
            }
        }
        tag.put("lost", ll);
        return tag;
    }

    private static StorageLedger load(CompoundTag tag, HolderLookup.Provider registries) {
        StorageLedger l = new StorageLedger();
        l.types.load(tag.getList("types", Tag.TAG_COMPOUND), tag.getInt("next_type"), registries);
        ListTag cl = tag.getList("cells", Tag.TAG_COMPOUND);
        for (int i = 0; i < cl.size(); i++) {
            CompoundTag c = cl.getCompound(i);
            if (!c.hasUUID("id")) continue;
            CellContents contents = new CellContents(CellTier.byId(c.getString("tier")));
            ListTag items = c.getList("items", Tag.TAG_COMPOUND);
            for (int j = 0; j < items.size(); j++) {
                CompoundTag it = items.getCompound(j);
                contents.putUnchecked(it.getInt("k"), it.getLong("n"));
            }
            l.cells.put(c.getUUID("id"), new CellRecord(contents, c.getLong("last_seen")));
        }
        ListTag tl = tag.getList("tokens", Tag.TAG_COMPOUND);
        for (int i = 0; i < tl.size(); i++) {
            CompoundTag c = tl.getCompound(i);
            l.tokens.putUnchecked(new TokenBook.Token(c.getString("id"), c.getInt("k"), c.getLong("n"),
                    c.getUUID("origin"), c.getUUID("issuer"), c.hasUUID("bound") ? c.getUUID("bound") : null,
                    c.getLong("expires")));
        }
        ListTag ll = tag.getList("lost", Tag.TAG_COMPOUND);
        for (int i = 0; i < ll.size(); i++) {
            CompoundTag c = ll.getCompound(i);
            l.tokens.putLostUnchecked(c.getUUID("issuer"), c.getInt("k"), c.getLong("n"));
        }
        return l;
    }

    /** Cell ids (tests and commands). */
    public Set<UUID> cellIds() {
        return Collections.unmodifiableSet(cells.keySet());
    }
}
//?}
