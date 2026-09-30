package com.example.evanscomputermod.sensor.wire;

//? if <=1.21.1 {
import com.example.evanscomputermod.block.TerminalBlock;
import com.example.evanscomputermod.sensor.TerminalWireHost;
import net.minecraft.core.BlockPos;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.phys.Vec3;
import org.jetbrains.annotations.NotNull;
import org.jetbrains.annotations.Nullable;

/**
 * A block with sensor-wire connection points (a lidar sensor, or a computer
 * with a Wired Sensor Module in a bay). Modelled on PowerGrid's
 * {@code IElectric} terminal handling, without the electrical parts.
 */
public interface IWireHost {

    /** Upper bound on terminal indices; {@link #terminal} may still return null for some. */
    int terminalCount();

    /** Terminal {@code index} in {@code state}, or null when it doesn't exist right now. */
    @Nullable
    WireTerminal terminal(BlockState state, int index);

    /** Terminal index at a block-local position, -1 if none. */
    default int terminalIndexAt(BlockState state, Vec3 local) {
        for(int i = 0; i < terminalCount(); ++i) {
            var terminal = terminal(state, i);
            if(terminal != null && terminal.check(local))
                return i;
        }
        return -1;
    }

    @Nullable
    static IWireHost getAt(Level level, BlockPos pos) {
        BlockState state = level.getBlockState(pos);
        if(state.getBlock() instanceof IWireHost host)
            return host;
        if(state.getBlock() instanceof TerminalBlock)
            return TerminalWireHost.INSTANCE;
        return null;
    }

    @NotNull
    static Vec3 getTerminalPos(Level level, BlockPos pos, int index) {
        var host = getAt(level, pos);
        if(host == null)
            return pos.getCenter();
        var terminal = host.terminal(level.getBlockState(pos), index);
        if(terminal == null)
            return pos.getCenter();
        var origin = terminal.getOrigin();
        return new Vec3(pos.getX() + origin.x, pos.getY() + origin.y, pos.getZ() + origin.z);
    }
}
//?}
