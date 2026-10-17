package com.example.evanscomputermod.computer.peripheral.generic;

//? if <=1.21.1 {
import com.example.evanscomputermod.api.peripheral.IComputerAccess;
import com.example.evanscomputermod.api.peripheral.IPeripheral;
import com.example.evanscomputermod.api.peripheral.PeripheralException;
import com.example.evanscomputermod.api.peripheral.PeripheralMethods;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.world.level.Level;
import net.neoforged.neoforge.capabilities.Capabilities;
import org.jetbrains.annotations.Nullable;

import java.util.ArrayList;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Set;

/**
 * Any block with NeoForge's item, fluid or energy capability, as a
 * peripheral: a chest, a tank or a battery from any mod works with a
 * computer next to it, with no code of its own. The type is the first of
 * {@code inventory}, {@code fluid_storage}, {@code energy_storage} the block
 * has; the methods are those of every capability it has
 * ({@link InventoryMethods}, {@link FluidMethods}, {@link EnergyMethods}).
 *
 * <p>Used only when the block offers no {@link
 * com.example.evanscomputermod.api.peripheral.PeripheralCapability peripheral}
 * of its own. The capabilities are looked up again on every call, so a block
 * that changes (or goes) is seen as it is now.
 */
public final class GenericPeripheral implements IPeripheral {
    public static final String INVENTORY = "inventory", FLUID_STORAGE = "fluid_storage", ENERGY_STORAGE = "energy_storage";

    private record Part(Object methods, PeripheralMethods table) {}

    private final Level level;
    private final BlockPos pos;
    private final Direction side;
    private final String type;
    private final List<Part> parts;
    private final Set<String> names = new LinkedHashSet<>();

    private GenericPeripheral(Level level, BlockPos pos, Direction side, String type, List<Part> parts) {
        this.level = level;
        this.pos = pos.immutable();
        this.side = side;
        this.type = type;
        this.parts = parts;
        for (Part p : parts) names.addAll(p.table().names());
    }

    /** The generic peripheral of the block at {@code pos}, seen from {@code side}, or null if it has none of the capabilities. */
    @Nullable
    public static GenericPeripheral of(Level level, BlockPos pos, Direction side) {
        List<Part> parts = new ArrayList<>();
        String type = null;
        if (level.getCapability(Capabilities.ItemHandler.BLOCK, pos, side) != null) {
            parts.add(new Part(new InventoryMethods(level, pos, side), PeripheralMethods.of(InventoryMethods.class)));
            type = INVENTORY;
        }
        if (level.getCapability(Capabilities.FluidHandler.BLOCK, pos, side) != null) {
            parts.add(new Part(new FluidMethods(level, pos, side), PeripheralMethods.of(FluidMethods.class)));
            if (type == null) type = FLUID_STORAGE;
        }
        if (level.getCapability(Capabilities.EnergyStorage.BLOCK, pos, side) != null) {
            parts.add(new Part(new EnergyMethods(level, pos, side), PeripheralMethods.of(EnergyMethods.class)));
            if (type == null) type = ENERGY_STORAGE;
        }
        return type == null ? null : new GenericPeripheral(level, pos, side, type, parts);
    }

    @Override
    public String getType() {
        return type;
    }

    @Override
    public Set<String> getMethodNames() {
        return names;
    }

    @Override
    @Nullable
    public Object callMethod(IComputerAccess computer, String method, Object[] arguments) throws PeripheralException {
        for (Part p : parts) {
            if (p.table().names().contains(method)) return p.table().call(p.methods(), computer, method, arguments);
        }
        throw new PeripheralException("no method " + method);
    }

    @Override
    public boolean isSame(IPeripheral other) {
        return other instanceof GenericPeripheral g && g.level == level && g.pos.equals(pos) && g.side == side
                && g.type.equals(type) && g.names.equals(names);
    }

    /** The inventory part, for another inventory moving items to or from this one; null if it has none. */
    @Nullable
    InventoryMethods inventory() {
        for (Part p : parts) if (p.methods() instanceof InventoryMethods i) return i;
        return null;
    }

    @Nullable
    FluidMethods fluids() {
        for (Part p : parts) if (p.methods() instanceof FluidMethods f) return f;
        return null;
    }

    /** The peripheral attached as {@code name}, which must be a generic one ({@code what}: "an inventory", ...). */
    static GenericPeripheral partner(IComputerAccess computer, String name, String what) throws PeripheralException {
        IPeripheral p = computer.getAvailablePeripheral(name);
        if (p == null) throw new PeripheralException("Target '" + name + "' does not exist");
        if (!(p instanceof GenericPeripheral g)) throw new PeripheralException("Target '" + name + "' is not " + what);
        return g;
    }
}
//?}
