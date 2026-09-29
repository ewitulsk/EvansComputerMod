package com.example.evanscomputermod.api.module;

import com.example.evanscomputermod.api.peripheral.IPeripheral;
import net.minecraft.nbt.CompoundTag;

/**
 * A peripheral that lives inside a computer's module bay. Created by an
 * {@link IComputerModuleItem} when the item is installed or the computer
 * loads, and attached to the computer under its slot name
 * ({@code left_bay_1}, {@code left_bay_2}, {@code right_bay_1}, {@code right_bay_2}).
 *
 * <p>Lifecycle (all on the server thread): {@link #onLoad} when it becomes
 * live in the world (installed, computer chunk loaded, after a Sable/Create
 * Aeronautics move), {@link #onUnload} when it leaves (ejected, block broken,
 * chunk unloaded, before a move). A module can be loaded and unloaded several
 * times; {@link #saveState} is called before every unload and before the
 * computer is saved.
 */
public interface IComputerModule extends IPeripheral {

    /** The module is live in the world at {@link IModuleHost#getPos()}. */
    default void onLoad() {
    }

    /** The module is leaving the world; release world registrations. */
    default void onUnload() {
    }

    /** Server tick while loaded. */
    default void tick() {
    }

    /**
     * Write persistent configuration into {@code tag}. It is stored on the
     * module's item stack, so it survives ejecting and reinstalling the module
     * and breaking the computer.
     */
    void saveState(CompoundTag tag);
}
