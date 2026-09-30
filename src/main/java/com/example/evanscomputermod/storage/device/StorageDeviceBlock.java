package com.example.evanscomputermod.storage.device;

//? if <=1.21.1 {

import com.example.evanscomputermod.sensor.wire.IWireHost;
import com.example.evanscomputermod.sensor.wire.WireConnections;
import com.example.evanscomputermod.sensor.wire.WireTerminal;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.world.item.context.BlockPlaceContext;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.EntityBlock;
import net.minecraft.world.level.block.HorizontalDirectionalBlock;
import net.minecraft.world.level.block.Mirror;
import net.minecraft.world.level.block.Rotation;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.block.entity.BlockEntityTicker;
import net.minecraft.world.level.block.entity.BlockEntityType;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.block.state.StateDefinition;
import org.jetbrains.annotations.Nullable;

import java.util.EnumMap;
import java.util.Map;

/**
 * Base of the Drive, Item Encoder and Item Decoder: a full block that faces
 * the player who placed it, has a Sensor Wire connector on its back (so a
 * Wired Bus Module can reach it), and drops what it holds when broken.
 */
public abstract class StorageDeviceBlock extends HorizontalDirectionalBlock implements EntityBlock, IWireHost {

    /** Connector nub in the middle of the back face (north-facing frame: back = south, z = 16). */
    private static final WireTerminal BACK = new WireTerminal(6, 6, 15, 10, 10, 17).withOrigin(8, 8, 16);
    private static final Map<Direction, WireTerminal> TERMINALS = new EnumMap<>(Direction.class);

    static {
        for (Direction d : Direction.Plane.HORIZONTAL) TERMINALS.put(d, BACK.rotateAroundY(WireTerminal.rotationFromNorth(d)));
    }

    protected StorageDeviceBlock(Properties properties) {
        super(properties);
        registerDefaultState(stateDefinition.any().setValue(FACING, Direction.NORTH));
    }

    @Override
    protected void createBlockStateDefinition(StateDefinition.Builder<Block, BlockState> builder) {
        builder.add(FACING);
    }

    @Nullable
    @Override
    public BlockState getStateForPlacement(BlockPlaceContext context) {
        return defaultBlockState().setValue(FACING, context.getHorizontalDirection().getOpposite());
    }

    @Override
    protected BlockState rotate(BlockState state, Rotation rotation) {
        return state.setValue(FACING, rotation.rotate(state.getValue(FACING)));
    }

    @Override
    protected BlockState mirror(BlockState state, Mirror mirror) {
        return state.rotate(mirror.getRotation(state.getValue(FACING)));
    }

    // ------------------------------------------------------------ wires

    @Override
    public int terminalCount() {
        return 1;
    }

    @Override
    @Nullable
    public WireTerminal terminal(BlockState state, int index) {
        if (index != 0 || !(state.getBlock() instanceof StorageDeviceBlock)) return null;
        return TERMINALS.get(state.getValue(FACING));
    }

    // ------------------------------------------------------------ block entity

    @Override
    protected void onRemove(BlockState state, Level level, BlockPos pos, BlockState newState, boolean movedByPiston) {
        if (!state.is(newState.getBlock())) {
            WireConnections.breakAll(level, pos, terminalCount());
            if (!level.isClientSide() && level.getBlockEntity(pos) instanceof StorageDeviceBlockEntity be) {
                be.dropContents();
            }
        }
        super.onRemove(state, level, pos, newState, movedByPiston);
    }

    @Nullable
    @Override
    @SuppressWarnings("unchecked")
    public <T extends BlockEntity> BlockEntityTicker<T> getTicker(Level level, BlockState state, BlockEntityType<T> type) {
        if (level.isClientSide()) return null;
        return (l, p, s, be) -> {
            if (be instanceof StorageDeviceBlockEntity d) d.serverTick();
        };
    }
}
//?}
