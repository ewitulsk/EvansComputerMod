package com.example.evanscomputermod.storage.device;

//? if <=1.21.1 {

import com.example.evanscomputermod.api.module.IComputerModule;
import com.example.evanscomputermod.api.module.IModuleHost;
import com.example.evanscomputermod.api.module.IModuleItemReceiver;
import com.example.evanscomputermod.api.peripheral.IComputerAccess;
import com.example.evanscomputermod.api.peripheral.PeripheralMethod;
import com.example.evanscomputermod.storage.api.StorageApiPeripheral;
import com.example.evanscomputermod.storage.api.StorageNet;
import com.example.evanscomputermod.storage.item.StorageCellItem;
import com.example.evanscomputermod.storage.ledger.StorageLedger;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.item.ItemStack;
import org.jetbrains.annotations.Nullable;

import java.util.Collection;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.UUID;

/**
 * Storage Module: one Storage Cell in a computer's bay, straight on the
 * computer (no drive needed). Right-click the bay slot with a cell to put it
 * in; sneak-right-click with an empty hand takes the cell out, then the
 * module. The cell travels with the module item.
 */
public final class StorageModule extends StorageApiPeripheral implements IComputerModule, IStorageDevice, IModuleItemReceiver {

    public static final String TYPE = "storage_module";

    private final IModuleHost host;
    private ItemStack cell = ItemStack.EMPTY;
    private UUID deviceId = UUID.randomUUID();
    private boolean loaded;

    private final CellMounts mounts = new CellMounts(new CellMounts.Host() {
        @Override
        @Nullable
        public ServerLevel level() {
            return host.getLevel();
        }

        @Override
        public boolean alive() {
            return loaded && host.isAlive();
        }

        @Override
        public IStorageDevice device() {
            return StorageModule.this;
        }

        @Override
        public ItemStack getStack(int slot) {
            return cell;
        }

        @Override
        public void stackChanged(int slot) {
            host.markDirty();
        }

        @Override
        public String describe(int slot) {
            return StorageModule.this.describe();
        }
    }, 1);

    public StorageModule(IModuleHost host, CompoundTag state) {
        this.host = host;
        if (state.hasUUID("device_id")) deviceId = state.getUUID("device_id");
        if (state.contains("cell")) {
            cell = ItemStack.parse(host.getLevel().registryAccess(), state.getCompound("cell")).orElse(ItemStack.EMPTY);
        }
    }

    public ItemStack cellStack() {
        return cell;
    }

    @Override
    public String getType() {
        return TYPE;
    }

    @Override
    @Nullable
    protected MinecraftServer server() {
        ServerLevel level = host.getLevel();
        return level == null ? null : level.getServer();
    }

    @Override
    public void contribute(StorageNet.Builder net, String attachment) {
        net.device(this);
    }

    // ------------------------------------------------------------ module lifecycle

    @Override
    public void onLoad() {
        loaded = true;
        mounts.start();
    }

    @Override
    public void onUnload() {
        mounts.stop();
        loaded = false;
    }

    @Override
    public void tick() {
        mounts.tick();
        ServerLevel level = host.getLevel();
        if (level == null) return;
        StorageLedger ledger = StorageLedger.get(level.getServer());
        if (ledger.anyChangedLastTick() && mounts.anyChanged(ledger)) postChanged(mounts.changedIds(ledger));
    }

    @Override
    public void saveState(CompoundTag tag) {
        tag.putUUID("device_id", deviceId);
        if (!cell.isEmpty()) tag.put("cell", cell.save(host.getLevel().registryAccess()));
    }

    // ------------------------------------------------------------ holding a cell

    @Override
    public boolean accepts(ItemStack stack) {
        return cell.isEmpty() && stack.getItem() instanceof StorageCellItem;
    }

    @Override
    public boolean insert(ItemStack stack) {
        if (!accepts(stack)) return false;
        cell = stack.copyWithCount(1);
        mounts.refresh(0);
        host.markDirty();
        return true;
    }

    @Override
    public ItemStack takeOut() {
        if (cell.isEmpty()) return ItemStack.EMPTY;
        mounts.stop();
        ItemStack out = cell;
        cell = ItemStack.EMPTY;
        if (loaded) mounts.start();
        host.markDirty();
        return out;
    }

    // ------------------------------------------------------------ IStorageDevice

    @Override
    public UUID deviceId() {
        return deviceId;
    }

    @Override
    public String kind() {
        return "module";
    }

    @Override
    public List<MountedCell> cells() {
        return mounts.mounted();
    }

    @Override
    public boolean isLive() {
        return loaded && host.isAlive() && mounts.isLive();
    }

    @Override
    public String describe() {
        return "Storage Module in " + host.getSlotName() + " at " + host.getPos().getX() + " "
                + host.getPos().getY() + " " + host.getPos().getZ();
    }

    @Override
    public Collection<IComputerAccess> watchers() {
        return computers;
    }

    @PeripheralMethod(description = "The module's id, its cell and whether the cell is mounted")
    public Map<String, Object> info() {
        Map<String, Object> m = new LinkedHashMap<>();
        m.put("id", deviceId.toString());
        UUID id = StorageCellItem.cellId(cell);
        m.put("cell", id == null ? null : id.toString());
        m.put("tier", cell.getItem() instanceof StorageCellItem c ? c.tier().label() : null);
        m.put("status", cell.isEmpty() ? "empty" : mounts.isDuplicate(0) ? "duplicate" : "mounted");
        return m;
    }
}
//?}
