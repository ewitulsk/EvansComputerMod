package com.example.evanscomputermod.block;

import com.example.evanscomputermod.computer.CableNetworkManager;
import com.mojang.serialization.MapCodec;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.world.item.context.BlockPlaceContext;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.util.RandomSource;
import net.minecraft.world.level.BlockGetter;
import net.minecraft.world.level.LevelReader;
//? if >=26.1 {
import net.minecraft.world.level.ScheduledTickAccess;
//?} else
/*import net.minecraft.world.level.LevelAccessor;*/
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.PipeBlock;
import net.minecraft.world.level.block.state.BlockBehaviour;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.block.state.StateDefinition;
import net.minecraft.world.level.block.state.properties.BooleanProperty;
import net.minecraft.world.phys.shapes.CollisionContext;
import net.minecraft.world.phys.shapes.Shapes;
import net.minecraft.world.phys.shapes.VoxelShape;
import org.jetbrains.annotations.Nullable;

/**
 * Network interface block that connects to cables, terminals, gateways, and other interface blocks.
 * Similar to NetworkCableBlock but visually distinct - used to provide additional network interfaces.
 */
public class InterfaceBlock extends Block {
    public static final BooleanProperty NORTH = PipeBlock.NORTH;
    public static final BooleanProperty SOUTH = PipeBlock.SOUTH;
    public static final BooleanProperty EAST = PipeBlock.EAST;
    public static final BooleanProperty WEST = PipeBlock.WEST;
    public static final BooleanProperty UP = PipeBlock.UP;
    public static final BooleanProperty DOWN = PipeBlock.DOWN;
    public static final MapCodec<InterfaceBlock> CODEC = simpleCodec(InterfaceBlock::new);

    private static final VoxelShape CENTER = Block.box(5, 5, 5, 11, 11, 11);
    private static final VoxelShape ARM_NORTH = Block.box(5, 5, 0, 11, 11, 5);
    private static final VoxelShape ARM_SOUTH = Block.box(5, 5, 11, 11, 11, 16);
    private static final VoxelShape ARM_EAST = Block.box(11, 5, 5, 16, 11, 11);
    private static final VoxelShape ARM_WEST = Block.box(0, 5, 5, 5, 11, 11);
    private static final VoxelShape ARM_UP = Block.box(5, 11, 5, 11, 16, 11);
    private static final VoxelShape ARM_DOWN = Block.box(5, 0, 5, 11, 5, 11);

    public InterfaceBlock(BlockBehaviour.Properties properties) {
        super(properties);
        this.registerDefaultState(this.stateDefinition.any()
            .setValue(NORTH, false).setValue(SOUTH, false)
            .setValue(EAST, false).setValue(WEST, false)
            .setValue(UP, false).setValue(DOWN, false));
    }

    @Override protected MapCodec<? extends Block> codec() { return CODEC; }

    @Override
    protected void createBlockStateDefinition(StateDefinition.Builder<Block, BlockState> builder) {
        builder.add(NORTH, SOUTH, EAST, WEST, UP, DOWN);
    }

    @Nullable @Override
    public BlockState getStateForPlacement(BlockPlaceContext context) {
        BlockGetter level = context.getLevel();
        BlockPos pos = context.getClickedPos();
        return this.defaultBlockState()
            .setValue(NORTH, canConnectToFaceAware(level.getBlockState(pos.north()), Direction.SOUTH))
            .setValue(SOUTH, canConnectToFaceAware(level.getBlockState(pos.south()), Direction.NORTH))
            .setValue(EAST, canConnectToFaceAware(level.getBlockState(pos.east()), Direction.WEST))
            .setValue(WEST, canConnectToFaceAware(level.getBlockState(pos.west()), Direction.EAST))
            .setValue(UP, canConnectToFaceAware(level.getBlockState(pos.above()), Direction.DOWN))
            .setValue(DOWN, canConnectToFaceAware(level.getBlockState(pos.below()), Direction.UP));
    }

    //? if >=26.1 {
    @Override
    protected BlockState updateShape(BlockState state, LevelReader level, ScheduledTickAccess ticks,
                                     BlockPos pos, Direction direction, BlockPos neighborPos,
                                     BlockState neighborState, RandomSource random) {
        return computeUpdateShape(state, direction, neighborState);
    }
    //?} else {
    /*@Override
    protected BlockState updateShape(BlockState state, Direction direction, BlockState neighborState,
                                     net.minecraft.world.level.LevelAccessor level, BlockPos pos, BlockPos neighborPos) {
        return computeUpdateShape(state, direction, neighborState);
    }*/
    //?}

    private BlockState computeUpdateShape(BlockState state, Direction direction, BlockState neighborState) {
        boolean connected;
        Block neighborBlock = neighborState.getBlock();
        if (neighborBlock instanceof TerminalBlock) {
            // Don't connect to terminal's screen face
            Direction facingToward = direction.getOpposite();
            Direction terminalFacing = neighborState.getValue(TerminalBlock.FACING);
            if (facingToward == terminalFacing) {
                connected = false;
            } else {
                connected = true;
            }
        } else {
            connected = canConnectTo(neighborState);
        }
        return state.setValue(getPropertyForDirection(direction), connected);
    }

    @Override
    protected VoxelShape getShape(BlockState state, BlockGetter level, BlockPos pos, CollisionContext context) {
        VoxelShape shape = CENTER;
        if (state.getValue(NORTH)) shape = Shapes.or(shape, ARM_NORTH);
        if (state.getValue(SOUTH)) shape = Shapes.or(shape, ARM_SOUTH);
        if (state.getValue(EAST)) shape = Shapes.or(shape, ARM_EAST);
        if (state.getValue(WEST)) shape = Shapes.or(shape, ARM_WEST);
        if (state.getValue(UP)) shape = Shapes.or(shape, ARM_UP);
        if (state.getValue(DOWN)) shape = Shapes.or(shape, ARM_DOWN);
        return shape;
    }

    @Override
    protected void onPlace(BlockState state, net.minecraft.world.level.Level level, BlockPos pos,
                           BlockState oldState, boolean movedByPiston) {
        super.onPlace(state, level, pos, oldState, movedByPiston);
        if (!level.isClientSide()) {
            CableNetworkManager mgr = CableNetworkManager.getInstance();
            if (mgr != null) mgr.invalidateCache();
        }
    }

    //? if >=26.1 {
    @Override
    protected void affectNeighborsAfterRemoval(BlockState state, ServerLevel level, BlockPos pos, boolean movedByPiston) {
        CableNetworkManager mgr = CableNetworkManager.getInstance();
        if (mgr != null) mgr.invalidateCache();
        super.affectNeighborsAfterRemoval(state, level, pos, movedByPiston);
    }
    //?} else {
    /*@Override
    protected void onRemove(BlockState state, net.minecraft.world.level.Level level, BlockPos pos, BlockState newState, boolean movedByPiston) {
        CableNetworkManager mgr = CableNetworkManager.getInstance();
        if (mgr != null) mgr.invalidateCache();
        super.onRemove(state, level, pos, newState, movedByPiston);
    }*/
    //?}

    /**
     * Face-aware connection check that blocks connection to a terminal's screen face.
     */
    private static boolean canConnectToFaceAware(BlockState state, Direction facingToward) {
        Block block = state.getBlock();
        if (block instanceof TerminalBlock) {
            Direction terminalFacing = state.getValue(TerminalBlock.FACING);
            if (facingToward == terminalFacing) {
                return false;
            }
            return true;
        }
        return canConnectTo(state);
    }

    public static boolean canConnectTo(BlockState state) {
        Block block = state.getBlock();
        return block instanceof InterfaceBlock
            || block instanceof NetworkCableBlock
            || block instanceof TerminalBlock
            || block instanceof InternetGatewayBlock;
    }

    private static BooleanProperty getPropertyForDirection(Direction direction) {
        return switch (direction) {
            case NORTH -> NORTH; case SOUTH -> SOUTH;
            case EAST -> EAST; case WEST -> WEST;
            case UP -> UP; case DOWN -> DOWN;
        };
    }
}
