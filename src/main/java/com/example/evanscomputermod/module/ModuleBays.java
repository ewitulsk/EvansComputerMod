package com.example.evanscomputermod.module;

import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.api.module.IComputerModule;
import com.example.evanscomputermod.api.module.IComputerModuleItem;
import com.example.evanscomputermod.api.module.IModuleHost;
import com.example.evanscomputermod.api.module.ModuleSlotVisual;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.Level;
import org.jetbrains.annotations.Nullable;

import java.util.ArrayList;
import java.util.Arrays;
import java.util.List;

/**
 * A computer's two module bays. Each bay opens when an expansion card is
 * installed and holds two modules: the left bay (on the computer's left side,
 * {@code facing.getCounterClockWise()}) has slots {@code left_bay_1} (upper)
 * and {@code left_bay_2} (lower); the right bay has {@code right_bay_1/2}.
 *
 * <p>Stacks are the source of truth and are saved; live
 * {@link IComputerModule}s exist only while the computer is loaded on the
 * server ({@link #load} / {@link #unload}).
 */
public final class ModuleBays {

    public static final int BAYS = 2;
    public static final int SLOTS = 4;
    public static final int LEFT = 0, RIGHT = 1;
    public static final String[] SLOT_NAMES = {"left_bay_1", "left_bay_2", "right_bay_1", "right_bay_2"};

    /** What the bays need from the computer block entity. */
    public interface Owner {
        @Nullable Level level();

        BlockPos pos();

        Direction facing();

        boolean isAlive();

        /** Bays or modules changed: save, and update the block's look. */
        void onBaysChanged();

        /** A live module's slot changed (installed or removed). */
        void onModuleChanged(String slotName, @Nullable IComputerModule module);
    }

    private final Owner owner;
    private int bays;
    private final ItemStack[] stacks = new ItemStack[SLOTS];
    private final IComputerModule[] live = new IComputerModule[SLOTS];
    private boolean loaded;

    public ModuleBays(Owner owner) {
        this.owner = owner;
        Arrays.fill(stacks, ItemStack.EMPTY);
    }

    public static int bayOf(int slot) {
        return slot / 2;
    }

    /** Side of the computer a bay opens on. */
    public static Direction baySide(Direction facing, int bay) {
        return bay == LEFT ? facing.getCounterClockWise() : facing.getClockWise();
    }

    /** Bay on {@code face}, or -1 if that face has no bay position. */
    public static int bayOnFace(Direction facing, Direction face) {
        if (face == baySide(facing, LEFT)) return LEFT;
        if (face == baySide(facing, RIGHT)) return RIGHT;
        return -1;
    }

    public boolean hasBay(int bay) {
        return (bays & (1 << bay)) != 0;
    }

    public ItemStack getStack(int slot) {
        return stacks[slot];
    }

    @Nullable
    public IComputerModule getModule(int slot) {
        return live[slot];
    }

    public boolean isLoaded() {
        return loaded;
    }

    public ModuleSlotVisual visual(int slot) {
        if (!hasBay(bayOf(slot)) || stacks[slot].isEmpty()) return ModuleSlotVisual.EMPTY;
        if (stacks[slot].getItem() instanceof IComputerModuleItem item) {
            return item.getSlotVisual(stacks[slot]);
        }
        return ModuleSlotVisual.GENERIC;
    }

    // ------------------------------------------------------------ persistence

    /** Snapshot for saving / the dropped item. Writes live module state into the stacks first. */
    public InstalledModules snapshot() {
        syncStates();
        return new InstalledModules(bays, Arrays.asList(stacks));
    }

    /** Replace contents (NBT load, or restoring from a placed item). Restarts live modules if loaded. */
    public void restore(InstalledModules data) {
        boolean wasLoaded = loaded;
        if (wasLoaded) unload();
        bays = data.bays();
        for (int i = 0; i < SLOTS; i++) stacks[i] = data.slots().get(i).copy();
        if (wasLoaded) load();
    }

    private void syncStates() {
        for (int i = 0; i < SLOTS; i++) {
            if (live[i] == null) continue;
            CompoundTag tag = new CompoundTag();
            try {
                live[i].saveState(tag);
            } catch (RuntimeException e) {
                EvansComputerMod.LOGGER.error("Module {} failed to save its state", SLOT_NAMES[i], e);
                continue;
            }
            ModuleState.write(stacks[i], tag);
        }
    }

    // ------------------------------------------------------------ lifecycle (server thread)

    /** Bring installed modules to life. Idempotent. */
    public void load() {
        if (loaded || !(owner.level() instanceof ServerLevel)) return;
        loaded = true;
        for (int i = 0; i < SLOTS; i++) start(i);
    }

    /** Save state and take all modules out of the world. Idempotent. */
    public void unload() {
        if (!loaded) return;
        syncStates();
        for (int i = 0; i < SLOTS; i++) stop(i);
        loaded = false;
    }

    public void tick() {
        if (!loaded) return;
        for (int i = 0; i < SLOTS; i++) {
            IComputerModule m = live[i];
            if (m == null) continue;
            try {
                m.tick();
            } catch (RuntimeException e) {
                EvansComputerMod.LOGGER.error("Module {} ({}) failed in tick; removing it from the world",
                        SLOT_NAMES[i], m.getType(), e);
                stop(i);
            }
        }
    }

    private void start(int slot) {
        if (live[slot] != null || !hasBay(bayOf(slot))) return;
        ItemStack stack = stacks[slot];
        if (stack.isEmpty() || !(stack.getItem() instanceof IComputerModuleItem item)) return;
        IComputerModule module;
        try {
            module = item.createModule(new Host(slot), stack, ModuleState.read(stack));
            module.onLoad();
        } catch (RuntimeException e) {
            EvansComputerMod.LOGGER.error("Module {} in {} failed to start", stack, SLOT_NAMES[slot], e);
            return;
        }
        live[slot] = module;
        owner.onModuleChanged(SLOT_NAMES[slot], module);
    }

    private void stop(int slot) {
        IComputerModule module = live[slot];
        if (module == null) return;
        live[slot] = null;
        owner.onModuleChanged(SLOT_NAMES[slot], null);
        try {
            module.onUnload();
        } catch (RuntimeException e) {
            EvansComputerMod.LOGGER.error("Module {} ({}) failed in onUnload", SLOT_NAMES[slot], module.getType(), e);
        }
    }

    // ------------------------------------------------------------ player actions (server thread)

    /**
     * Install an expansion card, preferring {@code preferredBay} (-1 = any).
     *
     * @return the bay it went into, or -1 if both bays are open
     */
    public int installCard(int preferredBay) {
        int bay = preferredBay >= 0 && !hasBay(preferredBay) ? preferredBay
                : !hasBay(LEFT) ? LEFT : !hasBay(RIGHT) ? RIGHT : -1;
        if (bay < 0) return -1;
        bays |= 1 << bay;
        owner.onBaysChanged();
        return bay;
    }

    /** Remove a bay's card if the bay is empty. */
    public boolean removeCard(int bay) {
        if (!hasBay(bay) || !stacks[bay * 2].isEmpty() || !stacks[bay * 2 + 1].isEmpty()) return false;
        bays &= ~(1 << bay);
        owner.onBaysChanged();
        return true;
    }

    /**
     * Install one of {@code stack} (which must be an {@link IComputerModuleItem}),
     * preferring {@code preferredSlot} (-1 = none), then the other slot of the
     * same bay, then any free slot.
     *
     * @return the slot, or -1 if no open slot is free
     */
    public int installModule(ItemStack stack, int preferredSlot) {
        List<Integer> order = new ArrayList<>();
        if (preferredSlot >= 0) {
            order.add(preferredSlot);
            order.add(preferredSlot ^ 1);
        }
        for (int i = 0; i < SLOTS; i++) order.add(i);
        for (int slot : order) {
            if (!hasBay(bayOf(slot)) || !stacks[slot].isEmpty()) continue;
            stacks[slot] = stack.copyWithCount(1);
            if (loaded) start(slot);
            owner.onBaysChanged();
            return slot;
        }
        return -1;
    }

    /** Take the module out of {@code slot}, with its state saved on the stack. */
    public ItemStack removeModule(int slot) {
        if (stacks[slot].isEmpty()) return ItemStack.EMPTY;
        if (live[slot] != null) {
            CompoundTag tag = new CompoundTag();
            try {
                live[slot].saveState(tag);
                ModuleState.write(stacks[slot], tag);
            } catch (RuntimeException e) {
                EvansComputerMod.LOGGER.error("Module {} failed to save its state", SLOT_NAMES[slot], e);
            }
            stop(slot);
        }
        ItemStack out = stacks[slot];
        stacks[slot] = ItemStack.EMPTY;
        owner.onBaysChanged();
        return out;
    }

    /** Number of installed expansion cards. */
    public int cardCount() {
        return Integer.bitCount(bays);
    }

    private final class Host implements IModuleHost {
        private final int slot;

        Host(int slot) {
            this.slot = slot;
        }

        @Override
        public ServerLevel getLevel() {
            return (ServerLevel) owner.level();
        }

        @Override
        public BlockPos getPos() {
            return owner.pos();
        }

        @Override
        public Direction getFacing() {
            return owner.facing();
        }

        @Override
        public int getSlot() {
            return slot;
        }

        @Override
        public String getSlotName() {
            return SLOT_NAMES[slot];
        }

        @Override
        public boolean isAlive() {
            return owner.isAlive() && loaded;
        }

        @Override
        public void markDirty() {
            owner.onBaysChanged();
        }
    }
}
