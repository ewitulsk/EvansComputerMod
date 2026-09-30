package com.example.evanscomputermod.sensor;

//? if <=1.21.1 {
import com.example.evanscomputermod.api.module.ModuleSlotVisual;
import com.example.evanscomputermod.block.TerminalBlock;
import com.example.evanscomputermod.module.ModuleBays;
import com.example.evanscomputermod.sensor.wire.IWireHost;
import com.example.evanscomputermod.sensor.wire.WireTerminal;
import net.minecraft.core.Direction;
import net.minecraft.world.level.block.state.BlockState;
import org.jetbrains.annotations.Nullable;

/**
 * The computer as a wire host: each bay slot holding a Wired Sensor Module
 * has one connection point, on the module's cartridge where it shows in the
 * bay (terminal index = slot index).
 */
public final class TerminalWireHost implements IWireHost {
    public static final TerminalWireHost INSTANCE = new TerminalWireHost();

    /** Per slot, in the north-facing frame (see scripts/gen-module-models.py: left bay x 0..3, right bay x 13..16). */
    private static final WireTerminal[] NORTH = new WireTerminal[ModuleBays.SLOTS];
    /** [facing 2d index][slot] */
    private static final WireTerminal[][] ROTATED = new WireTerminal[4][ModuleBays.SLOTS];

    static {
        for(int slot = 0; slot < ModuleBays.SLOTS; slot++) {
            boolean upper = slot % 2 == 0;
            int y0 = upper ? 10 : 2;
            boolean left = ModuleBays.bayOf(slot) == ModuleBays.LEFT;
            // The connector nub sits at z 9.5..12.5 on the cartridge face; the wire leaves just outside the block.
            NORTH[slot] = left
                    ? new WireTerminal(-0.5, y0 + 0.5, 9, 1, y0 + 3.5, 13).withOrigin(-0.5, y0 + 2, 11)
                    : new WireTerminal(15, y0 + 0.5, 9, 16.5, y0 + 3.5, 13).withOrigin(16.5, y0 + 2, 11);
        }
        for(Direction facing : Direction.Plane.HORIZONTAL) {
            for(int slot = 0; slot < ModuleBays.SLOTS; slot++) {
                ROTATED[facing.get2DDataValue()][slot] = NORTH[slot].rotateAroundY(WireTerminal.rotationFromNorth(facing));
            }
        }
    }

    private TerminalWireHost() {
    }

    @Override
    public int terminalCount() {
        return ModuleBays.SLOTS;
    }

    @Override
    @Nullable
    public WireTerminal terminal(BlockState state, int index) {
        if(index < 0 || index >= ModuleBays.SLOTS || !(state.getBlock() instanceof TerminalBlock))
            return null;
        if(state.getValue(TerminalBlock.SLOTS[index]) != ModuleSlotVisual.WIRED_SENSOR)
            return null;
        return ROTATED[state.getValue(TerminalBlock.FACING).get2DDataValue()][index];
    }
}
//?}
