package com.example.evanscomputermod.storage.core;

//? if <=1.21.1 {

import java.util.Collections;
import java.util.LinkedHashMap;
import java.util.Map;

/**
 * What one Storage Cell holds: item type ids ({@code keyId}, see
 * {@code ItemTypes}) to counts, plus the capacity rules of its
 * {@link CellTier}. No Minecraft types, so the accounting is plain Java.
 *
 * <p>Mutators either apply fully or throw {@link StorageException} and change
 * nothing.
 */
public final class CellContents {

    private final CellTier tier;
    private final LinkedHashMap<Integer, Long> counts = new LinkedHashMap<>();
    private long items;

    public CellContents(CellTier tier) {
        this.tier = tier;
    }

    public CellTier tier() {
        return tier;
    }

    public int types() {
        return counts.size();
    }

    public long items() {
        return items;
    }

    public boolean isEmpty() {
        return counts.isEmpty();
    }

    public long count(int key) {
        Long c = counts.get(key);
        return c == null ? 0 : c;
    }

    public boolean has(int key) {
        return counts.containsKey(key);
    }

    /** Stored counts, in insertion order. Read-only view. */
    public Map<Integer, Long> view() {
        return Collections.unmodifiableMap(counts);
    }

    public static long usedBytes(CellTier tier, int types, long items) {
        return types * tier.bytesPerType() + (items + CellTier.ITEMS_PER_BYTE - 1) / CellTier.ITEMS_PER_BYTE;
    }

    public long usedBytes() {
        return usedBytes(tier, counts.size(), items);
    }

    /** How many more of {@code key} fit. */
    public long maxInsert(int key) {
        int newTypes = counts.containsKey(key) ? counts.size() : counts.size() + 1;
        if (newTypes > CellTier.MAX_TYPES) return 0;
        long freeBytes = tier.bytes() - newTypes * tier.bytesPerType();
        if (freeBytes <= 0) return 0;
        long max = freeBytes * CellTier.ITEMS_PER_BYTE - items;
        return Math.max(0, max);
    }

    public void add(int key, long n) throws StorageException {
        if (n < 0) throw new StorageException("count must not be negative");
        if (n == 0) return;
        if (n > maxInsert(key)) throw new StorageException("cell is full");
        counts.merge(key, n, Long::sum);
        items += n;
    }

    public void remove(int key, long n) throws StorageException {
        if (n < 0) throw new StorageException("count must not be negative");
        if (n == 0) return;
        long have = count(key);
        if (have < n) throw new StorageException("not enough items (have " + have + ", need " + n + ")");
        if (have == n) counts.remove(key);
        else counts.put(key, have - n);
        items -= n;
    }

    /** Load without capacity checks (from saved data; the tier may have been reconfigured). */
    public void putUnchecked(int key, long n) {
        if (n <= 0) return;
        Long old = counts.put(key, n);
        items += n - (old == null ? 0 : old);
    }
}
//?}
