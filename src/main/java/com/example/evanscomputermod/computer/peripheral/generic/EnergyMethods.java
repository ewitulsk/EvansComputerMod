package com.example.evanscomputermod.computer.peripheral.generic;

//? if <=1.21.1 {
import com.example.evanscomputermod.api.peripheral.IComputerAccess;
import com.example.evanscomputermod.api.peripheral.IPeripheral;
import com.example.evanscomputermod.api.peripheral.PeripheralException;
import com.example.evanscomputermod.api.peripheral.PeripheralMethod;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.world.level.Level;
import net.neoforged.neoforge.capabilities.Capabilities;
import net.neoforged.neoforge.energy.IEnergyStorage;

/**
 * The {@code energy_storage} methods of a {@link GenericPeripheral} (any block
 * with NeoForge's energy capability, in FE), as ComputerCraft has them.
 */
public final class EnergyMethods implements IPeripheral {
    private final Level level;
    private final BlockPos pos;
    private final Direction side;

    EnergyMethods(Level level, BlockPos pos, Direction side) {
        this.level = level;
        this.pos = pos;
        this.side = side;
    }

    @Override public String getType() { return GenericPeripheral.ENERGY_STORAGE; }
    @Override public java.util.Set<String> getMethodNames() { return java.util.Set.of(); }
    @Override public Object callMethod(IComputerAccess c, String m, Object[] a) throws PeripheralException {
        throw new PeripheralException("no method " + m);
    }

    private IEnergyStorage handler() throws PeripheralException {
        IEnergyStorage h = level.getCapability(Capabilities.EnergyStorage.BLOCK, pos, side);
        if (h == null) throw new PeripheralException("the energy storage is gone");
        return h;
    }

    @PeripheralMethod(description = "Energy stored, in FE")
    public int getEnergy() throws PeripheralException {
        return handler().getEnergyStored();
    }

    @PeripheralMethod(description = "Energy it can hold, in FE")
    public int getEnergyCapacity() throws PeripheralException {
        return handler().getMaxEnergyStored();
    }
}
//?}
