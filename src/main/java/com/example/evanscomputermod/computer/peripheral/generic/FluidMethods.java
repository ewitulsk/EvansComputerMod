package com.example.evanscomputermod.computer.peripheral.generic;

//? if <=1.21.1 {
import com.example.evanscomputermod.api.peripheral.IComputerAccess;
import com.example.evanscomputermod.api.peripheral.IPeripheral;
import com.example.evanscomputermod.api.peripheral.PeripheralException;
import com.example.evanscomputermod.api.peripheral.PeripheralMethod;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.resources.ResourceLocation;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.material.Fluid;
import net.minecraft.world.level.material.Fluids;
import net.neoforged.neoforge.capabilities.Capabilities;
import net.neoforged.neoforge.fluids.FluidStack;
import net.neoforged.neoforge.fluids.FluidUtil;
import net.neoforged.neoforge.fluids.capability.IFluidHandler;
import org.jetbrains.annotations.Nullable;

import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

/**
 * The {@code fluid_storage} methods of a {@link GenericPeripheral} (any block
 * with NeoForge's fluid capability), as ComputerCraft has them. Amounts are
 * in millibuckets (1000 = a bucket).
 */
public final class FluidMethods implements IPeripheral {
    private final Level level;
    private final BlockPos pos;
    private final Direction side;

    FluidMethods(Level level, BlockPos pos, Direction side) {
        this.level = level;
        this.pos = pos;
        this.side = side;
    }

    @Override public String getType() { return GenericPeripheral.FLUID_STORAGE; }
    @Override public java.util.Set<String> getMethodNames() { return java.util.Set.of(); }
    @Override public Object callMethod(IComputerAccess c, String m, Object[] a) throws PeripheralException {
        throw new PeripheralException("no method " + m);
    }

    IFluidHandler handler() throws PeripheralException {
        IFluidHandler h = level.getCapability(Capabilities.FluidHandler.BLOCK, pos, side);
        if (h == null) throw new PeripheralException("the tank is gone");
        return h;
    }

    @PeripheralMethod(description = "The fluid in each tank: a list of {name, amount} (amount in mB; empty tanks left out)")
    public List<Object> tanks() throws PeripheralException {
        IFluidHandler h = handler();
        List<Object> out = new ArrayList<>();
        for (int i = 0; i < h.getTanks(); i++) {
            FluidStack f = h.getFluidInTank(i);
            if (f.isEmpty()) continue;
            Map<String, Object> m = new LinkedHashMap<>();
            m.put("name", BuiltInRegistries.FLUID.getKey(f.getFluid()).toString());
            m.put("amount", f.getAmount());
            out.add(m);
        }
        return out;
    }

    @PeripheralMethod(description = "Move fluid (up to limit mB, only fluid_name if given) into the tank attached as to_name: mB moved")
    public int pushFluid(IComputerAccess computer, String toName, @Nullable Integer limit, @Nullable String fluidName)
            throws PeripheralException {
        return move(handler(), target(computer, toName).handler(), limit, fluidName);
    }

    @PeripheralMethod(description = "Move fluid (up to limit mB, only fluid_name if given) from the tank attached as from_name: mB moved")
    public int pullFluid(IComputerAccess computer, String fromName, @Nullable Integer limit, @Nullable String fluidName)
            throws PeripheralException {
        return move(target(computer, fromName).handler(), handler(), limit, fluidName);
    }

    private static FluidMethods target(IComputerAccess computer, String name) throws PeripheralException {
        FluidMethods f = GenericPeripheral.partner(computer, name, "a tank").fluids();
        if (f == null) throw new PeripheralException("Target '" + name + "' is not a tank");
        return f;
    }

    private static int move(IFluidHandler from, IFluidHandler to, @Nullable Integer limit, @Nullable String fluidName)
            throws PeripheralException {
        int max = limit == null ? Integer.MAX_VALUE : limit;
        if (max < 0) throw new PeripheralException("Limit must be 0 or more");
        if (max == 0) return 0;
        if (fluidName == null) return FluidUtil.tryFluidTransfer(to, from, max, true).getAmount();
        ResourceLocation id = ResourceLocation.tryParse(fluidName);
        Fluid fluid = id == null ? Fluids.EMPTY : BuiltInRegistries.FLUID.get(id);
        if (fluid == Fluids.EMPTY) throw new PeripheralException("No such fluid " + fluidName);
        return FluidUtil.tryFluidTransfer(to, from, new FluidStack(fluid, max), true).getAmount();
    }
}
//?}
