package com.example.evanscomputermod.storage.device;

//? if <=1.21.1 {

import com.example.evanscomputermod.api.peripheral.AnnotatedPeripheral;
import com.example.evanscomputermod.api.peripheral.IComputerAccess;
import com.example.evanscomputermod.api.peripheral.PeripheralException;
import com.example.evanscomputermod.api.peripheral.PeripheralMethod;
import com.example.evanscomputermod.storage.StorageContent;
import com.example.evanscomputermod.storage.StorageRules;
import com.example.evanscomputermod.storage.api.StorageApiPeripheral;
import com.example.evanscomputermod.storage.api.StorageNet;
import com.example.evanscomputermod.storage.core.StorageException;
import com.example.evanscomputermod.storage.ledger.ItemTypes;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.core.HolderLookup;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.block.HorizontalDirectionalBlock;
import net.minecraft.world.level.block.state.BlockState;
import net.neoforged.neoforge.capabilities.Capabilities;
import net.neoforged.neoforge.items.IItemHandler;
import net.neoforged.neoforge.items.ItemHandlerHelper;
import net.neoforged.neoforge.items.ItemStackHandler;
import org.jetbrains.annotations.Nullable;

import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

/**
 * Item Decoder: the only way items leave the ledger. {@link #capacityFor}
 * says how many fit in its buffer, the ledger is debited, then
 * {@link #deliver} creates exactly those items. Every tick the buffer is
 * pushed into the inventory in front; hoppers and funnels may also pull from
 * it (but never insert).
 */
public class DecoderBlockEntity extends StorageDeviceBlockEntity implements IItemPort {

    public static final String TYPE = "item_decoder";
    public static final int BUFFER_SLOTS = 9;

    private final ItemStackHandler buffer = new ItemStackHandler(BUFFER_SLOTS) {
        @Override
        public int getSlotLimit(int slot) {
            return 99;
        }

        @Override
        protected void onContentsChanged(int slot) {
            setChanged();
        }
    };

    /** What automation sees: extract only. */
    private final IItemHandler outside = new IItemHandler() {
        @Override
        public int getSlots() {
            return BUFFER_SLOTS;
        }

        @Override
        public ItemStack getStackInSlot(int slot) {
            return buffer.getStackInSlot(slot);
        }

        @Override
        public ItemStack insertItem(int slot, ItemStack stack, boolean simulate) {
            return stack;
        }

        @Override
        public ItemStack extractItem(int slot, int amount, boolean simulate) {
            return buffer.extractItem(slot, amount, simulate);
        }

        @Override
        public int getSlotLimit(int slot) {
            return buffer.getSlotLimit(slot);
        }

        @Override
        public boolean isItemValid(int slot, ItemStack stack) {
            return false;
        }
    };

    private final Peripheral peripheral = new Peripheral();
    private long decoded;

    public DecoderBlockEntity(BlockPos pos, BlockState state) {
        super(StorageContent.DECODER_BE.get(), pos, state);
    }

    public Peripheral peripheral() {
        return peripheral;
    }

    public IItemHandler itemHandler() {
        return outside;
    }

    public ItemStackHandler buffer() {
        return buffer;
    }

    @Override
    public String kind() {
        return "decoder";
    }

    @Override
    public boolean isLive() {
        return isStarted();
    }

    @Override
    protected Iterable<ItemStack> takeDrops() {
        List<ItemStack> out = new ArrayList<>();
        for (int i = 0; i < BUFFER_SLOTS; i++) {
            ItemStack s = buffer.getStackInSlot(i);
            if (!s.isEmpty()) {
                out.add(s.copy());
                buffer.setStackInSlot(i, ItemStack.EMPTY);
            }
        }
        return out;
    }

    // ------------------------------------------------------------ decoding (server thread)

    /** How many of {@code proto} (up to {@code want}) fit in the buffer right now. */
    public long capacityFor(ItemStack proto, long want) {
        if (!isStarted()) return 0;
        int max = proto.getMaxStackSize();
        long room = 0;
        for (int i = 0; i < BUFFER_SLOTS && room < want; i++) {
            ItemStack s = buffer.getStackInSlot(i);
            if (s.isEmpty()) room += max;
            else if (ItemStack.isSameItemSameComponents(s, proto)) room += Math.max(0, max - s.getCount());
        }
        return Math.min(room, want);
    }

    /** Create {@code count} of {@code proto} in the buffer; the caller checked {@link #capacityFor} and debited the ledger. */
    public void deliver(ItemStack proto, long count) {
        long left = count;
        int max = proto.getMaxStackSize();
        for (int i = 0; i < BUFFER_SLOTS && left > 0; i++) {
            ItemStack s = buffer.getStackInSlot(i);
            if (!s.isEmpty() && ItemStack.isSameItemSameComponents(s, proto) && s.getCount() < max) {
                int n = (int) Math.min(left, max - s.getCount());
                buffer.setStackInSlot(i, s.copyWithCount(s.getCount() + n));
                left -= n;
            }
        }
        for (int i = 0; i < BUFFER_SLOTS && left > 0; i++) {
            if (buffer.getStackInSlot(i).isEmpty()) {
                int n = (int) Math.min(left, max);
                buffer.setStackInSlot(i, proto.copyWithCount(n));
                left -= n;
            }
        }
        if (left > 0) {
            // Can't happen when capacityFor was honoured; never lose items silently.
            while (left > 0 && level != null) {
                int n = (int) Math.min(left, max);
                net.minecraft.world.Containers.dropItemStack(level, worldPosition.getX() + 0.5,
                        worldPosition.getY() + 1.0, worldPosition.getZ() + 0.5, proto.copyWithCount(n));
                left -= n;
            }
        }
        decoded += count;
        push();
    }

    @Override
    protected void tick() {
        if (level != null && level.getGameTime() % 4 == 0) push();
    }

    /** Move buffered items into the inventory in front. */
    private void push() {
        if (!(level instanceof ServerLevel sl)) return;
        Direction out = getBlockState().getValue(HorizontalDirectionalBlock.FACING);
        IItemHandler target = sl.getCapability(Capabilities.ItemHandler.BLOCK, worldPosition.relative(out), out.getOpposite());
        if (target == null) return;
        for (int i = 0; i < BUFFER_SLOTS; i++) {
            ItemStack s = buffer.getStackInSlot(i);
            if (s.isEmpty()) continue;
            ItemStack left = ItemHandlerHelper.insertItemStacked(target, s.copy(), false);
            if (left.getCount() != s.getCount()) buffer.setStackInSlot(i, left);
        }
    }

    // ------------------------------------------------------------ persistence

    @Override
    protected void saveAdditional(CompoundTag tag, HolderLookup.Provider registries) {
        super.saveAdditional(tag, registries);
        tag.put("buffer", buffer.serializeNBT(registries));
        tag.putLong("decoded", decoded);
    }

    @Override
    protected void loadAdditional(CompoundTag tag, HolderLookup.Provider registries) {
        super.loadAdditional(tag, registries);
        if (tag.contains("buffer")) buffer.deserializeNBT(registries, tag.getCompound("buffer"));
        decoded = tag.getLong("decoded");
    }

    // ------------------------------------------------------------ peripheral

    /** Peripheral type {@code item_decoder}. */
    public final class Peripheral extends AnnotatedPeripheral implements StorageNet.Node {
        @Override
        public String getType() {
            return TYPE;
        }

        @Override
        public void contribute(StorageNet.Builder net, String attachment) {
            net.port(attachment, DecoderBlockEntity.this);
        }

        @PeripheralMethod(description = "Send up to count of an item (by key) out of this decoder: extract(key, count, cell)")
        public long extract(IComputerAccess computer, String key, long count, @Nullable String cell) throws PeripheralException {
            if (level == null || level.getServer() == null) throw new StorageException("not loaded");
            int k = ItemTypes.parseKey(key);
            if (k == 0) throw new StorageException("bad item key '" + key + "'");
            StorageNet net = StorageNet.of(computer, level.getServer());
            return StorageApiPeripheral.extractTo(net, DecoderBlockEntity.this, k, count, cell);
        }

        @PeripheralMethod(description = "Items waiting in the buffer")
        public List<Map<String, Object>> buffer() {
            List<Map<String, Object>> out = new ArrayList<>();
            for (int i = 0; i < BUFFER_SLOTS; i++) {
                ItemStack s = DecoderBlockEntity.this.buffer.getStackInSlot(i);
                if (s.isEmpty()) continue;
                Map<String, Object> m = new LinkedHashMap<>();
                m.put("slot", i);
                m.put("id", StorageRules.itemId(s));
                m.put("name", StorageRules.displayName(s));
                m.put("count", s.getCount());
                out.add(m);
            }
            return out;
        }

        @PeripheralMethod(description = "Id and items decoded so far")
        public Map<String, Object> status() {
            Map<String, Object> m = new LinkedHashMap<>();
            m.put("id", deviceId.toString());
            m.put("decoded", decoded);
            int used = 0;
            for (int i = 0; i < BUFFER_SLOTS; i++) if (!DecoderBlockEntity.this.buffer.getStackInSlot(i).isEmpty()) used++;
            m.put("buffer_slots_used", used);
            return m;
        }
    }
}
//?}
