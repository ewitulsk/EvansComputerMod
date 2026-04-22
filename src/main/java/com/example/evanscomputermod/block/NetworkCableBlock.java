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
//?}
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
 * Network cable block that visually connects to adjacent cables, terminals, and the internet gateway.
 * Uses 6 BooleanProperties to track connections in each direction.
 * No block entity needed — purely visual + network topology.
 */
public class NetworkCableBlock extends Block
        //? if <=1.21.1 {
        implements dev.ryanhcode.sable.api.block.BlockSubLevelAssemblyListener
        //?}
{

    //? if <=1.21.1 {
    @Override
    public void afterMove(net.minecraft.server.level.ServerLevel from,
                          net.minecraft.server.level.ServerLevel to,
                          BlockState state, BlockPos oldPos, BlockPos newPos) {
        com.example.evanscomputermod.sable.SableAssemblyHooks.onCableAfterMove(from, to, state, oldPos, newPos);
    }
    //?}

    public static final BooleanProperty NORTH = PipeBlock.NORTH;
    public static final BooleanProperty SOUTH = PipeBlock.SOUTH;
    public static final BooleanProperty EAST = PipeBlock.EAST;
    public static final BooleanProperty WEST = PipeBlock.WEST;
    public static final BooleanProperty UP = PipeBlock.UP;
    public static final BooleanProperty DOWN = PipeBlock.DOWN;

    public static final MapCodec<NetworkCableBlock> CODEC = simpleCodec(NetworkCableBlock::new);

    // VoxelShapes: center + directional arms
    private static final VoxelShape CENTER = Block.box(6, 6, 6, 10, 10, 10);
    private static final VoxelShape ARM_NORTH = Block.box(6, 6, 0, 10, 10, 6);
    private static final VoxelShape ARM_SOUTH = Block.box(6, 6, 10, 10, 10, 16);
    private static final VoxelShape ARM_EAST = Block.box(10, 6, 6, 16, 10, 10);
    private static final VoxelShape ARM_WEST = Block.box(0, 6, 6, 6, 10, 10);
    private static final VoxelShape ARM_UP = Block.box(6, 10, 6, 10, 16, 10);
    private static final VoxelShape ARM_DOWN = Block.box(6, 0, 6, 10, 6, 10);

    public NetworkCableBlock(BlockBehaviour.Properties properties) {
        super(properties);
        this.registerDefaultState(this.stateDefinition.any()
                .setValue(NORTH, false)
                .setValue(SOUTH, false)
                .setValue(EAST, false)
                .setValue(WEST, false)
                .setValue(UP, false)
                .setValue(DOWN, false));
    }

    @Override
    protected MapCodec<? extends Block> codec() {
        return CODEC;
    }

    @Override
    protected void createBlockStateDefinition(StateDefinition.Builder<Block, BlockState> builder) {
        builder.add(NORTH, SOUTH, EAST, WEST, UP, DOWN);
    }

    @Nullable
    @Override
    public BlockState getStateForPlacement(BlockPlaceContext context) {
        BlockGetter level = context.getLevel();
        BlockPos pos = context.getClickedPos();
        return this.defaultBlockState()
                .setValue(NORTH, canConnectToFace(level.getBlockState(pos.north()), Direction.SOUTH, level, pos.north()))
                .setValue(SOUTH, canConnectToFace(level.getBlockState(pos.south()), Direction.NORTH, level, pos.south()))
                .setValue(EAST, canConnectToFace(level.getBlockState(pos.east()), Direction.WEST, level, pos.east()))
                .setValue(WEST, canConnectToFace(level.getBlockState(pos.west()), Direction.EAST, level, pos.west()))
                .setValue(UP, canConnectToFace(level.getBlockState(pos.above()), Direction.DOWN, level, pos.above()))
                .setValue(DOWN, canConnectToFace(level.getBlockState(pos.below()), Direction.UP, level, pos.below()));
    }

    //? if >=26.1 {
    @Override
    protected BlockState updateShape(BlockState state, LevelReader level, ScheduledTickAccess ticks,
                                     BlockPos pos, Direction direction, BlockPos neighborPos,
                                     BlockState neighborState, RandomSource random) {
        boolean connected = canConnectToFace(neighborState, direction.getOpposite(), level, neighborPos);
        return state.setValue(getPropertyForDirection(direction), connected);
    }
    //?} else {
    /*@Override
    protected BlockState updateShape(BlockState state, Direction direction, BlockState neighborState,
                                     net.minecraft.world.level.LevelAccessor level, BlockPos pos, BlockPos neighborPos) {
        boolean connected = canConnectToFace(neighborState, direction.getOpposite(), level, neighborPos);
        return state.setValue(getPropertyForDirection(direction), connected);
    }*/
    //?}

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
            if (mgr != null) {
                mgr.invalidateCache();
            }
        }
    }

    //? if >=26.1 {
    @Override
    protected void affectNeighborsAfterRemoval(BlockState state, ServerLevel level, BlockPos pos, boolean movedByPiston) {
        CableNetworkManager mgr = CableNetworkManager.getInstance();
        if (mgr != null) {
            mgr.invalidateCache();
        }
        super.affectNeighborsAfterRemoval(state, level, pos, movedByPiston);
    }
    //?} else {
    /*@Override
    protected void onRemove(BlockState state, net.minecraft.world.level.Level level, BlockPos pos, BlockState newState, boolean movedByPiston) {
        CableNetworkManager mgr = CableNetworkManager.getInstance();
        if (mgr != null) {
            mgr.invalidateCache();
        }
        super.onRemove(state, level, pos, newState, movedByPiston);
    }*/
    //?}

    /**
     * Check if this cable can connect to the given neighbor block state.
     */
    public static boolean canConnectTo(BlockState state) {
        Block block = state.getBlock();
        return block instanceof NetworkCableBlock
                || block instanceof TerminalBlock
                || block instanceof InternetGatewayBlock
                || block instanceof InterfaceBlock;
    }

    /**
     * Face-aware connection check that also considers disabled faces on terminals.
     * @param state the neighbor block state
     * @param facingToward the direction from the cable toward the neighbor (i.e., the face of the neighbor being connected to)
     * @param level the block getter for accessing block entities
     * @param neighborPos the position of the neighbor block
     */
    public static boolean canConnectToFace(BlockState state, Direction facingToward, BlockGetter level, BlockPos neighborPos) {
        Block block = state.getBlock();
        if (block instanceof NetworkCableBlock || block instanceof InternetGatewayBlock || block instanceof InterfaceBlock) {
            return true;
        }
        if (block instanceof TerminalBlock) {
            // Don't connect to the screen face
            Direction terminalFacing = state.getValue(TerminalBlock.FACING);
            if (facingToward == terminalFacing) {
                return false;
            }
            // Don't connect to disabled (link-down) faces
            if (level != null && level.getBlockEntity(neighborPos) instanceof TerminalBlockEntity tbe) {
                return !tbe.isFaceDisabled(facingToward.ordinal());
            }
            return true;
        }
        return false;
    }

    public static BooleanProperty getPropertyForDirection(Direction direction) {
        return switch (direction) {
            case NORTH -> NORTH;
            case SOUTH -> SOUTH;
            case EAST -> EAST;
            case WEST -> WEST;
            case UP -> UP;
            case DOWN -> DOWN;
        };
    }
}
