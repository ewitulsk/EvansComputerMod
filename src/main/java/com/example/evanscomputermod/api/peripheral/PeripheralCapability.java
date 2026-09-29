package com.example.evanscomputermod.api.peripheral;

import com.example.evanscomputermod.EvansComputerMod;
import net.minecraft.core.Direction;
import net.neoforged.neoforge.capabilities.BlockCapability;
import org.jetbrains.annotations.Nullable;

/**
 * Block capability through which blocks next to a computer expose a
 * peripheral. Register a provider in {@code RegisterCapabilitiesEvent}:
 * <pre>
 * event.registerBlockEntity(PeripheralCapability.PERIPHERAL, MY_BE_TYPE.get(),
 *         (be, side) -> be.getPeripheral());
 * </pre>
 * The context is the face of the block being queried (the one touching the
 * computer); it is never {@code null} when a computer asks.
 */
public final class PeripheralCapability {

    public static final BlockCapability<IPeripheral, @Nullable Direction> PERIPHERAL =
            BlockCapability.createSided(EvansComputerMod.id("peripheral"), IPeripheral.class);

    private PeripheralCapability() {
    }
}
