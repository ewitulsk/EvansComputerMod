package com.example.evanscomputermod.storage.device;

//? if <=1.21.1 {

import com.example.evanscomputermod.storage.StorageContent;
import com.example.evanscomputermod.storage.StorageEvents;
import com.example.evanscomputermod.storage.core.CellContents;
import com.example.evanscomputermod.storage.core.CellTier;
import com.example.evanscomputermod.storage.item.CellSummary;
import com.example.evanscomputermod.storage.item.StorageCellItem;
import com.example.evanscomputermod.storage.ledger.MountRef;
import com.example.evanscomputermod.storage.ledger.StorageLedger;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.item.ItemStack;
import org.jetbrains.annotations.Nullable;

import java.util.ArrayList;
import java.util.List;
import java.util.UUID;

/**
 * Mounts the cells in a device's slots into the ledger while the device is
 * live in the world: gives new cells an id, refuses a cell whose id is
 * already live elsewhere (a copy), and keeps each item's summary current for
 * tooltips. Shared by the Drive and the bay Storage Module. Server thread.
 */
public final class CellMounts {

    /** What the mounts need from the device. */
    public interface Host {
        @Nullable ServerLevel level();

        /** The device's block entity / module is in the world. */
        boolean alive();

        IStorageDevice device();

        ItemStack getStack(int slot);

        /** A stack's components changed (id given, summary updated): persist. */
        void stackChanged(int slot);

        String describe(int slot);
    }

    private final class Slot implements MountRef {
        final int index;
        @Nullable UUID id;
        @Nullable CellTier tier;
        boolean mounted;
        boolean duplicate;

        Slot(int index) {
            this.index = index;
        }

        @Override
        public boolean isAlive() {
            return live && host.alive() && mounted;
        }

        @Override
        public String describe() {
            return host.describe(index);
        }
    }

    private final Host host;
    private final Slot[] slots;
    private boolean live;
    private int ticks;

    public CellMounts(Host host, int size) {
        this.host = host;
        this.slots = new Slot[size];
        for (int i = 0; i < size; i++) slots[i] = new Slot(i);
    }

    public int size() {
        return slots.length;
    }

    public boolean isLive() {
        return live;
    }

    @Nullable
    private StorageLedger ledger() {
        ServerLevel level = host.level();
        return level == null ? null : StorageLedger.get(level.getServer());
    }

    /** The device is in the world: mount every cell. */
    public void start() {
        if (live || ledger() == null) return;
        live = true;
        StorageEvents.deviceLive(host.device(), true);
        for (int i = 0; i < slots.length; i++) refresh(i);
    }

    /** The device is leaving the world: unmount everything (summaries written). */
    public void stop() {
        if (!live) return;
        for (int i = 0; i < slots.length; i++) unmount(i);
        live = false;
        StorageEvents.deviceLive(host.device(), false);
    }

    /** A slot's stack may have changed. */
    public void refresh(int index) {
        if (!live) return;
        ItemStack stack = host.getStack(index);
        CellTier tier = StorageCellItem.tierOf(stack);
        Slot s = slots[index];
        UUID want = tier == null ? null : stack.get(StorageContent.CELL_ID.get());
        if (tier != null && s.mounted && want != null && want.equals(s.id) && tier == s.tier) return;
        unmount(index);
        if (tier == null) return;
        StorageLedger ledger = ledger();
        if (ledger == null) return;
        if (want == null) {
            want = StorageCellItem.ensureId(stack);
            host.stackChanged(index);
        }
        s.id = want;
        s.tier = tier;
        if (ledger.tryMount(want, s)) {
            ledger.ensureCell(want, tier);
            s.mounted = true;
            s.duplicate = false;
            writeSummary(ledger, index);
        } else {
            s.duplicate = true;
        }
    }

    private void unmount(int index) {
        Slot s = slots[index];
        StorageLedger ledger = ledger();
        if (s.mounted && s.id != null && ledger != null) {
            writeSummary(ledger, index);
            ledger.unmount(s.id, s);
        }
        s.mounted = false;
        s.duplicate = false;
        s.id = null;
        s.tier = null;
    }

    /** Copy the ledger's fill onto the item (for tooltips). */
    private void writeSummary(StorageLedger ledger, int index) {
        Slot s = slots[index];
        if (s.id == null) return;
        ItemStack stack = host.getStack(index);
        if (!s.id.equals(StorageCellItem.cellId(stack))) return;
        CellContents c = ledger.get(s.id);
        CellSummary summary = c == null ? new CellSummary(0, 0, 0) : new CellSummary(c.types(), c.items(), c.usedBytes());
        if (!summary.equals(stack.get(StorageContent.CELL_SUMMARY.get()))) {
            stack.set(StorageContent.CELL_SUMMARY.get(), summary);
            host.stackChanged(index);
        }
    }

    /** Server tick: refresh summaries of changed cells, retry duplicates whose original went away. */
    public void tick() {
        if (!live) return;
        StorageLedger ledger = ledger();
        if (ledger == null) return;
        if (ledger.anyChangedLastTick()) {
            for (Slot s : slots) {
                if (s.mounted && s.id != null && ledger.changedLastTick(s.id)) writeSummary(ledger, s.index);
            }
        }
        if (++ticks >= 20) {
            ticks = 0;
            for (Slot s : slots) {
                if (s.duplicate) {
                    int i = s.index;
                    s.duplicate = false;
                    s.id = null;
                    refresh(i);
                }
            }
        }
    }

    public List<MountedCell> mounted() {
        List<MountedCell> out = new ArrayList<>();
        for (Slot s : slots) {
            if (s.mounted && s.id != null && s.tier != null) out.add(new MountedCell(s.id, s.tier, host.device(), s.index));
        }
        return out;
    }

    public boolean isDuplicate(int index) {
        return slots[index].duplicate;
    }

    public boolean isMounted(int index) {
        return slots[index].mounted;
    }

    /** Whether any mounted cell changed last tick. */
    public boolean anyChanged(StorageLedger ledger) {
        for (Slot s : slots) if (s.mounted && s.id != null && ledger.changedLastTick(s.id)) return true;
        return false;
    }

    /** Ids of mounted cells that changed last tick. */
    public List<String> changedIds(StorageLedger ledger) {
        List<String> out = new ArrayList<>();
        for (Slot s : slots) if (s.mounted && s.id != null && ledger.changedLastTick(s.id)) out.add(s.id.toString());
        return out;
    }
}
//?}
