package com.example.evanscomputermod.radio.conductor;

//? if <=1.21.1 {
import com.example.evanscomputermod.radio.RadioContent;
import com.example.evanscomputermod.radio.antenna.AntennaManager;
import com.example.evanscomputermod.sensor.wire.IWireHost;
import com.example.evanscomputermod.sensor.wire.WireConnections;
import com.example.evanscomputermod.sensor.wire.WireTerminal;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.world.item.context.BlockPlaceContext;
import net.minecraft.world.level.BlockGetter;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.Rotation;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.block.state.StateDefinition;
import net.minecraft.world.level.block.state.properties.BlockStateProperties;
import net.minecraft.world.level.block.state.properties.EnumProperty;
import net.minecraft.world.phys.shapes.CollisionContext;
import net.minecraft.world.phys.shapes.Shapes;
import net.minecraft.world.phys.shapes.VoxelShape;
import org.jetbrains.annotations.Nullable;

import java.util.EnumMap;
import java.util.Map;

/**
 * The feed point: a two-sided insulator with a coax port, the centre of a
 * dipole or the base of a vertical. Its {@link #AXIS} picks the two antenna
 * terminals (the faces along the axis, shown as lugs); the four other sides
 * are the coax port. Fine Wire attaches to the two lugs (wire-host terminals
 * 0 = negative side, 1 = positive side), so VHF/UHF antennas can be built
 * from routed wire.
 *
 * <p>Placed on the ground with a vertical axis and nothing conductive below,
 * it feeds a monopole against the ground.
 */
public class FeedPointBlock extends ConductorBlock implements IWireHost {
    public static final EnumProperty<Direction.Axis> AXIS = BlockStateProperties.AXIS;
    private static final Map<Direction, WireTerminal> TERMINALS = new EnumMap<>(Direction.class);
    private static final Map<Direction.Axis, VoxelShape> LUGS = new EnumMap<>(Direction.Axis.class);

    static {
        TERMINALS.put(Direction.WEST, new WireTerminal(-1, 6, 6, 1, 10, 10));
        TERMINALS.put(Direction.EAST, new WireTerminal(15, 6, 6, 17, 10, 10));
        TERMINALS.put(Direction.DOWN, new WireTerminal(6, -1, 6, 10, 1, 10));
        TERMINALS.put(Direction.UP, new WireTerminal(6, 15, 6, 10, 17, 10));
        TERMINALS.put(Direction.NORTH, new WireTerminal(6, 6, -1, 10, 10, 1));
        TERMINALS.put(Direction.SOUTH, new WireTerminal(6, 6, 15, 10, 10, 17));
        LUGS.put(Direction.Axis.X, Block.box(0, 6.5, 6.5, 16, 9.5, 9.5));
        LUGS.put(Direction.Axis.Y, Block.box(6.5, 0, 6.5, 9.5, 16, 9.5));
        LUGS.put(Direction.Axis.Z, Block.box(6.5, 6.5, 0, 9.5, 9.5, 16));
    }

    public FeedPointBlock(Properties properties) {
        super(properties, Role.FEED, 8, true);
    }

    @Override
    protected BlockState defaultState(BlockState s) {
        return s.setValue(AXIS, Direction.Axis.X);
    }

    @Override
    protected void createBlockStateDefinition(StateDefinition.Builder<Block, BlockState> builder) {
        super.createBlockStateDefinition(builder);
        builder.add(AXIS);
    }

    public static Direction armSide(BlockState state, boolean positive) {
        return Direction.get(positive ? Direction.AxisDirection.POSITIVE : Direction.AxisDirection.NEGATIVE, state.getValue(AXIS));
    }

    @Override
    public SideKind sideKind(BlockState state, Direction side) {
        return side.getAxis() == state.getValue(AXIS) ? SideKind.BARE : SideKind.COAX;
    }

    /**
     * A vertical feed point standing on a solid metal block feeds against it: the block is
     * the ground plane (counterpoise) of a monopole, not part of the antenna, so the lower lug
     * doesn't join it. Thin metal (iron bars, chains, a lightning rod) under it still joins as
     * a lower element.
     */
    @Override
    public boolean connects(BlockGetter level, BlockPos pos, BlockState state, Direction side) {
        if (side == Direction.DOWN && state.getValue(AXIS) == Direction.Axis.Y) {
            BlockPos below = pos.below();
            if (isMetalGround(level, below, level.getBlockState(below))) return false;
        }
        return super.connects(level, pos, state, side);
    }

    /** A full, solid {@code #rf_conductors} block (not a wire/feedline block): metal that acts as ground under an antenna. */
    public static boolean isMetalGround(BlockGetter level, BlockPos pos, BlockState s) {
        return !(s.getBlock() instanceof ConductorBlock) && s.is(RadioContent.RF_CONDUCTORS)
                && !s.is(RadioContent.RF_INSULATORS) && s.isCollisionShapeFullBlock(level, pos);
    }

    /**
     * Axis: along conductors already next to the spot if there are any (so a
     * feed dropped into a gap in a wire lines up with it), else the clicked
     * face's axis, like a log.
     */
    @Override
    protected @Nullable BlockState placementState(BlockPlaceContext ctx) {
        BlockGetter level = ctx.getLevel();
        BlockPos pos = ctx.getClickedPos();
        for (Direction.Axis axis : new Direction.Axis[] {ctx.getClickedFace().getAxis(), Direction.Axis.X, Direction.Axis.Z, Direction.Axis.Y}) {
            for (Direction.AxisDirection ad : Direction.AxisDirection.values()) {
                BlockState n = level.getBlockState(pos.relative(Direction.get(ad, axis)));
                if (n.getBlock() instanceof ConductorBlock c && c.role() == Role.CONDUCTOR)
                    return defaultBlockState().setValue(AXIS, axis);
            }
        }
        return defaultBlockState().setValue(AXIS, ctx.getClickedFace().getAxis());
    }

    @Override
    protected VoxelShape getShape(BlockState state, BlockGetter level, BlockPos pos, CollisionContext context) {
        return Shapes.or(super.getShape(state, level, pos, context), LUGS.get(state.getValue(AXIS)));
    }

    @Override
    protected BlockState rotate(BlockState state, Rotation rotation) {
        BlockState s = super.rotate(state, rotation);
        if (rotation == Rotation.CLOCKWISE_90 || rotation == Rotation.COUNTERCLOCKWISE_90) {
            Direction.Axis a = state.getValue(AXIS);
            if (a == Direction.Axis.X) s = s.setValue(AXIS, Direction.Axis.Z);
            else if (a == Direction.Axis.Z) s = s.setValue(AXIS, Direction.Axis.X);
        }
        return s;
    }

    // ------------------------------------------------------------ fine wire terminals

    @Override
    public int terminalCount() {
        return 2;
    }

    @Override
    public @Nullable WireTerminal terminal(BlockState state, int index) {
        if (!(state.getBlock() instanceof FeedPointBlock) || index < 0 || index > 1) return null;
        return TERMINALS.get(armSide(state, index == 1));
    }

    @Override
    protected void onRemove(BlockState state, Level level, BlockPos pos, BlockState newState, boolean movedByPiston) {
        if (!level.isClientSide() && !state.is(newState.getBlock())) {
            WireConnections.breakAll(level, pos, terminalCount());
            AntennaManager.forget(level, pos);
            if (level instanceof net.minecraft.server.level.ServerLevel sl)
                com.example.evanscomputermod.radio.hazard.RadioOwners.remove(sl, pos);
        }
        super.onRemove(state, level, pos, newState, movedByPiston);
    }
}
//?}
