package com.example.evanscomputermod.storage.device;

//? if <=1.21.1 {

import com.example.evanscomputermod.api.peripheral.AnnotatedPeripheral;
import com.example.evanscomputermod.api.peripheral.IComputerAccess;
import com.example.evanscomputermod.api.peripheral.PeripheralException;
import com.example.evanscomputermod.api.peripheral.PeripheralMethod;
import com.example.evanscomputermod.storage.StorageContent;
import com.example.evanscomputermod.storage.StorageRules;
import com.example.evanscomputermod.storage.api.StorageNet;
import com.example.evanscomputermod.storage.core.CellContents;
import com.example.evanscomputermod.storage.core.StorageException;
import com.example.evanscomputermod.storage.ledger.StorageLedger;
import net.minecraft.core.BlockPos;
import net.minecraft.core.HolderLookup;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.nbt.ListTag;
import net.minecraft.nbt.StringTag;
import net.minecraft.nbt.Tag;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.block.state.BlockState;
import net.neoforged.neoforge.items.IItemHandler;
import org.jetbrains.annotations.Nullable;

import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.UUID;

/**
 * Item Encoder: items put in (by hoppers, funnels, belts, or a player's
 * right-click) are destroyed and credited to a cell it can reach. Nothing is
 * buffered: an insert either lands in the ledger in the same call or is handed
 * back, so a full network pushes back on the hopper.
 *
 * <p>Which cell: a program's {@code set_target}, else AE2's order (a cell that
 * already has this item, then any with room; higher drive priority first).
 */
public class EncoderBlockEntity extends StorageDeviceBlockEntity implements IItemPort {

    public static final String TYPE = "item_encoder";
    private static final int REACH_REFRESH_TICKS = 20;

    private List<IStorageDevice> reach = List.of();
    private int reachTicks;
    @Nullable
    private UUID target;
    /** Allowed item ids or keys; empty = everything. */
    private final Set<String> filter = new LinkedHashSet<>();
    private long encoded;

    private final Peripheral peripheral = new Peripheral();
    private final IItemHandler handler = new Handler();

    public EncoderBlockEntity(BlockPos pos, BlockState state) {
        super(StorageContent.ENCODER_BE.get(), pos, state);
    }

    public Peripheral peripheral() {
        return peripheral;
    }

    public IItemHandler itemHandler() {
        return handler;
    }

    @Override
    public String kind() {
        return "encoder";
    }

    @Override
    public boolean isLive() {
        return isStarted();
    }

    @Override
    protected void start() {
        refreshReach();
    }

    @Override
    protected void tick() {
        if (++reachTicks >= REACH_REFRESH_TICKS) refreshReach();
    }

    private void refreshReach() {
        reachTicks = 0;
        reach = level instanceof ServerLevel sl ? LocalReach.find(sl, worldPosition) : List.of();
    }

    public List<IStorageDevice> reach() {
        return reach;
    }

    @Override
    protected Iterable<ItemStack> takeDrops() {
        return List.of();
    }

    // ------------------------------------------------------------ encoding (server thread)

    @Nullable
    private StorageLedger ledger() {
        return level instanceof ServerLevel sl ? StorageLedger.get(sl.getServer()) : null;
    }

    /**
     * Encode as much of {@code stack} as fits. Returns what is left.
     *
     * @param simulate only work out how much would fit
     */
    public ItemStack encode(ItemStack stack, boolean simulate) {
        if (stack.isEmpty() || !isStarted()) return stack;
        StorageLedger ledger = ledger();
        if (ledger == null || StorageRules.whyNotEncodable(stack, ledger) != null) return stack;
        if (!passesFilter(stack, ledger)) return stack;
        int key = ledger.types().find(stack);
        List<CellContents> order = targets(ledger, key);
        int probe = key == 0 ? -1 : key;
        long left = stack.getCount();
        long fits = 0;
        for (CellContents c : order) {
            fits += Math.min(left - fits, c.maxInsert(probe));
            if (fits >= left) break;
        }
        if (fits <= 0) return stack;
        if (simulate) return stack.copyWithCount((int) (left - fits));
        try {
            key = ledger.intern(stack);
        } catch (StorageException e) {
            return stack;
        }
        long todo = fits;
        for (MountedCell cell : targetCells(ledger, key)) {
            if (todo <= 0) break;
            CellContents c = ledger.get(cell.id());
            if (c == null) continue;
            long n = Math.min(todo, c.maxInsert(key));
            if (n <= 0) continue;
            try {
                c.add(key, n);
            } catch (StorageException e) {
                continue;
            }
            ledger.changed(cell.id());
            todo -= n;
        }
        long done = fits - todo;
        encoded += done;
        setChanged();
        return stack.copyWithCount((int) (left - done));
    }

    private boolean passesFilter(ItemStack stack, StorageLedger ledger) {
        if (filter.isEmpty()) return true;
        if (filter.contains(StorageRules.itemId(stack))) return true;
        int key = ledger.types().find(stack);
        return key != 0 && filter.contains(com.example.evanscomputermod.storage.ledger.ItemTypes.keyOf(key));
    }

    private List<CellContents> targets(StorageLedger ledger, int key) {
        List<CellContents> out = new ArrayList<>();
        for (MountedCell c : targetCells(ledger, key)) {
            CellContents contents = ledger.get(c.id());
            if (contents != null) out.add(contents);
        }
        return out;
    }

    /** Cells in fill order: the program's target only, else ones already holding the item, then the rest. */
    private List<MountedCell> targetCells(StorageLedger ledger, int key) {
        List<MountedCell> all = StorageNet.ofDevices(ledger, reach).cells();
        if (target != null) {
            for (MountedCell c : all) if (c.id().equals(target)) return List.of(c);
            return List.of();
        }
        List<MountedCell> first = new ArrayList<>();
        List<MountedCell> rest = new ArrayList<>();
        for (MountedCell c : all) {
            CellContents contents = ledger.get(c.id());
            if (contents == null) continue;
            if (key != 0 && contents.has(key)) first.add(c);
            else rest.add(c);
        }
        first.addAll(rest);
        return first;
    }

    private final class Handler implements IItemHandler {
        @Override
        public int getSlots() {
            return 1;
        }

        @Override
        public ItemStack getStackInSlot(int slot) {
            return ItemStack.EMPTY;
        }

        @Override
        public ItemStack insertItem(int slot, ItemStack stack, boolean simulate) {
            return encode(stack, simulate);
        }

        @Override
        public ItemStack extractItem(int slot, int amount, boolean simulate) {
            return ItemStack.EMPTY;
        }

        @Override
        public int getSlotLimit(int slot) {
            return 64;
        }

        @Override
        public boolean isItemValid(int slot, ItemStack stack) {
            StorageLedger ledger = ledger();
            return ledger != null && StorageRules.whyNotEncodable(stack, ledger) == null;
        }
    }

    // ------------------------------------------------------------ persistence

    @Override
    protected void saveAdditional(CompoundTag tag, HolderLookup.Provider registries) {
        super.saveAdditional(tag, registries);
        if (target != null) tag.putUUID("target", target);
        ListTag f = new ListTag();
        for (String s : filter) f.add(StringTag.valueOf(s));
        tag.put("filter", f);
        tag.putLong("encoded", encoded);
    }

    @Override
    protected void loadAdditional(CompoundTag tag, HolderLookup.Provider registries) {
        super.loadAdditional(tag, registries);
        target = tag.hasUUID("target") ? tag.getUUID("target") : null;
        filter.clear();
        ListTag f = tag.getList("filter", Tag.TAG_STRING);
        for (int i = 0; i < f.size(); i++) filter.add(f.getString(i));
        encoded = tag.getLong("encoded");
    }

    // ------------------------------------------------------------ peripheral

    /** Peripheral type {@code item_encoder}: pick the target cell and an item filter. */
    public final class Peripheral extends AnnotatedPeripheral implements StorageNet.Node {
        @Override
        public String getType() {
            return TYPE;
        }

        @Override
        public void contribute(StorageNet.Builder net, String attachment) {
            net.port(attachment, EncoderBlockEntity.this);
        }

        @PeripheralMethod(description = "Send everything to one cell (id), or None for automatic")
        public void setTarget(IComputerAccess computer, @Nullable String cell) throws PeripheralException {
            if (cell == null || cell.isEmpty()) {
                target = null;
            } else {
                try {
                    target = UUID.fromString(cell);
                } catch (IllegalArgumentException e) {
                    throw new StorageException("cell must be a full cell id");
                }
            }
            setChanged();
        }

        @PeripheralMethod(description = "Only accept these item ids or keys (a list), or None for everything")
        public void setFilter(@Nullable List<Object> items) throws PeripheralException {
            filter.clear();
            if (items != null) {
                for (Object o : items) {
                    if (!(o instanceof String s)) throw new StorageException("filter entries must be strings");
                    filter.add(s);
                }
            }
            setChanged();
        }

        @PeripheralMethod(description = "Target, filter, reachable cells and items encoded so far")
        public Map<String, Object> status() {
            Map<String, Object> m = new LinkedHashMap<>();
            m.put("id", deviceId.toString());
            m.put("target", target == null ? null : target.toString());
            m.put("filter", new ArrayList<>(filter));
            StorageLedger ledger = ledger();
            List<String> cells = new ArrayList<>();
            if (ledger != null) for (MountedCell c : StorageNet.ofDevices(ledger, reach).cells()) cells.add(c.id().toString());
            m.put("cells", cells);
            m.put("encoded", encoded);
            return m;
        }
    }
}
//?}
