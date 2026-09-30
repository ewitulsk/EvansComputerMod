package com.example.evanscomputermod.storage.device;

//? if <=1.21.1 {

import com.example.evanscomputermod.api.peripheral.IComputerAccess;
import com.example.evanscomputermod.api.peripheral.PeripheralMethod;
import com.example.evanscomputermod.storage.StorageContent;
import com.example.evanscomputermod.storage.api.StorageApiPeripheral;
import com.example.evanscomputermod.storage.api.StorageNet;
import com.example.evanscomputermod.storage.item.StorageCellItem;
import com.example.evanscomputermod.storage.ledger.StorageLedger;
import net.minecraft.core.BlockPos;
import net.minecraft.core.HolderLookup;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.network.chat.Component;
import net.minecraft.network.protocol.Packet;
import net.minecraft.network.protocol.game.ClientGamePacketListener;
import net.minecraft.network.protocol.game.ClientboundBlockEntityDataPacket;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.MenuProvider;
import net.minecraft.world.entity.player.Inventory;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.inventory.AbstractContainerMenu;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.state.BlockState;
import net.neoforged.neoforge.items.ItemStackHandler;
import org.jetbrains.annotations.Nullable;

import java.util.ArrayList;
import java.util.Collection;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.UUID;

/**
 * Drive: holds up to {@link #SLOTS} Storage Cells and mounts them while it is
 * in the world. Computers reach it when it touches them or through a Wired Bus
 * Module; either way it exposes the storage API as peripheral type
 * {@code storage_drive}.
 */
public class DriveBlockEntity extends StorageDeviceBlockEntity implements IStorageDevice, MenuProvider {

    public static final int SLOTS = 10;
    public static final String TYPE = "storage_drive";

    /** Slot status sent to clients for the renderer: 0 empty, 1 mounted, 2 duplicate (refused). */
    public static final int STATUS_EMPTY = 0, STATUS_OK = 1, STATUS_DUPLICATE = 2;

    private final ItemStackHandler cells = new ItemStackHandler(SLOTS) {
        @Override
        public boolean isItemValid(int slot, ItemStack stack) {
            return stack.getItem() instanceof StorageCellItem;
        }

        @Override
        public int getSlotLimit(int slot) {
            return 1;
        }

        @Override
        protected void onContentsChanged(int slot) {
            mounts.refresh(slot);
            changed();
        }
    };

    private final CellMounts mounts = new CellMounts(new CellMounts.Host() {
        @Override
        @Nullable
        public ServerLevel level() {
            return level instanceof ServerLevel sl ? sl : null;
        }

        @Override
        public boolean alive() {
            return isStarted();
        }

        @Override
        public IStorageDevice device() {
            return DriveBlockEntity.this;
        }

        @Override
        public ItemStack getStack(int slot) {
            return cells.getStackInSlot(slot);
        }

        @Override
        public void stackChanged(int slot) {
            changed();
        }

        @Override
        public String describe(int slot) {
            return DriveBlockEntity.this.describe() + ", slot " + (slot + 1);
        }
    }, SLOTS);

    private final Peripheral peripheral = new Peripheral();
    private int priority;
    private boolean syncPending;
    private final int[] clientStatus = new int[SLOTS];

    public DriveBlockEntity(BlockPos pos, BlockState state) {
        super(StorageContent.DRIVE_BE.get(), pos, state);
    }

    public ItemStackHandler cellHandler() {
        return cells;
    }

    public Peripheral peripheral() {
        return peripheral;
    }

    public CellMounts mounts() {
        return mounts;
    }

    private void changed() {
        setChanged();
        syncPending = true;
    }

    // ------------------------------------------------------------ lifecycle

    @Override
    protected void start() {
        mounts.start();
        syncPending = true;
    }

    @Override
    protected void stop() {
        mounts.stop();
    }

    @Override
    protected void tick() {
        mounts.tick();
        StorageLedger ledger = level instanceof ServerLevel sl ? StorageLedger.get(sl.getServer()) : null;
        if (ledger != null && ledger.anyChangedLastTick() && mounts.anyChanged(ledger)) {
            List<String> ids = mounts.changedIds(ledger);
            peripheral.post(ids);
        }
        for (int i = 0; i < SLOTS; i++) {
            int s = status(i);
            if (s != clientStatus[i]) {
                clientStatus[i] = s;
                syncPending = true;
            }
        }
        if (syncPending && level != null && level.getGameTime() % 4 == 0) {
            syncPending = false;
            BlockState st = getBlockState();
            level.sendBlockUpdated(worldPosition, st, st, Block.UPDATE_CLIENTS);
        }
    }

    public int status(int slot) {
        if (cells.getStackInSlot(slot).isEmpty()) return STATUS_EMPTY;
        if (mounts.isDuplicate(slot)) return STATUS_DUPLICATE;
        return STATUS_OK;
    }

    /** Client: status the server last sent. */
    public int clientStatus(int slot) {
        return clientStatus[slot];
    }

    @Override
    protected Iterable<ItemStack> takeDrops() {
        List<ItemStack> out = new ArrayList<>();
        for (int i = 0; i < SLOTS; i++) {
            ItemStack s = cells.getStackInSlot(i);
            if (!s.isEmpty()) {
                out.add(s.copy());
                cells.setStackInSlot(i, ItemStack.EMPTY);
            }
        }
        return out;
    }

    // ------------------------------------------------------------ IStorageDevice

    @Override
    public String kind() {
        return "drive";
    }

    @Override
    public List<MountedCell> cells() {
        return mounts.mounted();
    }

    @Override
    public int priority() {
        return priority;
    }

    @Override
    public boolean isLive() {
        return isStarted() && mounts.isLive();
    }

    @Override
    public Collection<IComputerAccess> watchers() {
        return peripheral.watchers();
    }

    // ------------------------------------------------------------ menu

    @Override
    public Component getDisplayName() {
        return Component.translatable("block.evanscomputermod.storage_drive");
    }

    @Override
    public AbstractContainerMenu createMenu(int id, Inventory inventory, Player player) {
        return new DriveMenu(id, inventory, this);
    }

    // ------------------------------------------------------------ persistence / sync

    @Override
    protected void saveAdditional(CompoundTag tag, HolderLookup.Provider registries) {
        super.saveAdditional(tag, registries);
        tag.put("cells", cells.serializeNBT(registries));
        tag.putInt("priority", priority);
    }

    @Override
    protected void loadAdditional(CompoundTag tag, HolderLookup.Provider registries) {
        super.loadAdditional(tag, registries);
        if (tag.contains("cells")) cells.deserializeNBT(registries, tag.getCompound("cells"));
        priority = tag.getInt("priority");
        if (tag.contains("status")) {
            int[] s = tag.getIntArray("status");
            for (int i = 0; i < SLOTS && i < s.length; i++) clientStatus[i] = s[i];
        }
        if (level != null && !level.isClientSide()) {
            for (int i = 0; i < SLOTS; i++) mounts.refresh(i);
        }
    }

    @Override
    public CompoundTag getUpdateTag(HolderLookup.Provider registries) {
        CompoundTag tag = new CompoundTag();
        tag.put("cells", cells.serializeNBT(registries));
        int[] s = new int[SLOTS];
        for (int i = 0; i < SLOTS; i++) s[i] = status(i);
        tag.putIntArray("status", s);
        return tag;
    }

    @Override
    @Nullable
    public Packet<ClientGamePacketListener> getUpdatePacket() {
        return ClientboundBlockEntityDataPacket.create(this);
    }

    // ------------------------------------------------------------ peripheral

    /** The drive as a peripheral: the storage API plus drive settings. */
    public final class Peripheral extends StorageApiPeripheral {
        @Override
        public String getType() {
            return TYPE;
        }

        @Override
        @Nullable
        protected MinecraftServer server() {
            return level == null ? null : level.getServer();
        }

        @Override
        public void contribute(StorageNet.Builder net, String attachment) {
            net.device(DriveBlockEntity.this);
        }

        void post(List<String> ids) {
            postChanged(ids);
        }

        @PeripheralMethod(description = "This drive's id and its slots: cell id and status per slot")
        public Map<String, Object> info() {
            Map<String, Object> m = new LinkedHashMap<>();
            m.put("id", deviceId.toString());
            m.put("priority", priority);
            List<Object> slots = new ArrayList<>();
            for (int i = 0; i < SLOTS; i++) {
                ItemStack s = cells.getStackInSlot(i);
                Map<String, Object> slot = new LinkedHashMap<>();
                UUID id = StorageCellItem.cellId(s);
                slot.put("cell", id == null ? null : id.toString());
                slot.put("tier", s.getItem() instanceof StorageCellItem c ? c.tier().label() : null);
                slot.put("status", switch (status(i)) {
                    case STATUS_OK -> "mounted";
                    case STATUS_DUPLICATE -> "duplicate";
                    default -> "empty";
                });
                slots.add(slot);
            }
            m.put("slots", slots);
            return m;
        }

        @PeripheralMethod(description = "Set the drive's priority (higher fills first)")
        public void setPriority(int value) {
            priority = value;
            setChanged();
        }

        @Override
        public boolean isSame(com.example.evanscomputermod.api.peripheral.IPeripheral other) {
            return other == this;
        }
    }
}
//?}
