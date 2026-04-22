package com.example.evanscomputermod.block;

import com.mojang.serialization.MapCodec;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.world.item.context.BlockPlaceContext;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.BaseEntityBlock;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.HorizontalDirectionalBlock;
import net.minecraft.world.level.block.Mirror;
import net.minecraft.world.level.block.RenderShape;
import net.minecraft.world.level.block.Rotation;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.block.state.BlockBehaviour;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.block.state.StateDefinition;
import net.minecraft.world.level.block.state.properties.EnumProperty;
//? if >=26.1 {
import net.minecraft.world.level.redstone.Orientation;
//?}
import net.minecraft.server.level.ServerLevel;

import org.jetbrains.annotations.Nullable;

/**
 * Screen block — an in-world display that mirrors a connected computer's screen output.
 * Multiple adjacent screens with the same facing connected to the same terminal
 * form a rectangular cluster whose resolution scales with the tile count.
 */
public class ScreenBlock extends BaseEntityBlock
        //? if <=1.21.1 {
        implements dev.ryanhcode.sable.api.block.BlockSubLevelAssemblyListener
        //?}
{

    /**
     * Rotate the FACING property so that bulk-move transactions (sable
     * physics-body assembly, structure blocks, etc.) produce a correctly
     * oriented screen on the destination side. Default {@code Block.rotate}
     * returns the state unchanged.
     */
    @Override
    protected BlockState rotate(BlockState state, Rotation rotation) {
        return state.setValue(FACING, rotation.rotate(state.getValue(FACING)));
    }

    @Override
    protected BlockState mirror(BlockState state, Mirror mirror) {
        return state.rotate(mirror.getRotation(state.getValue(FACING)));
    }

    //? if <=1.21.1 {
    @Override
    public void afterMove(net.minecraft.server.level.ServerLevel from,
                          net.minecraft.server.level.ServerLevel to,
                          BlockState state, BlockPos oldPos, BlockPos newPos) {
        com.example.evanscomputermod.sable.SableAssemblyHooks.onScreenAfterMove(from, to, state, oldPos, newPos);
    }
    //?}

    public static final EnumProperty<Direction> FACING = HorizontalDirectionalBlock.FACING;
    public static final MapCodec<ScreenBlock> CODEC = simpleCodec(ScreenBlock::new);

    public ScreenBlock(BlockBehaviour.Properties properties) {
        super(properties);
        this.registerDefaultState(this.stateDefinition.any().setValue(FACING, Direction.NORTH));
    }

    @Override
    protected MapCodec<? extends BaseEntityBlock> codec() {
        return CODEC;
    }

    @Override
    protected void createBlockStateDefinition(StateDefinition.Builder<Block, BlockState> builder) {
        builder.add(FACING);
    }

    @Nullable
    @Override
    public BlockState getStateForPlacement(BlockPlaceContext context) {
        return this.defaultBlockState().setValue(FACING, context.getHorizontalDirection().getOpposite());
    }

    @Nullable
    @Override
    public BlockEntity newBlockEntity(BlockPos pos, BlockState state) {
        return new ScreenBlockEntity(pos, state);
    }

    @Override
    protected RenderShape getRenderShape(BlockState state) {
        return RenderShape.MODEL;
    }

    @Override
    protected void onPlace(BlockState state, Level level, BlockPos pos, BlockState oldState, boolean movedByPiston) {
        super.onPlace(state, level, pos, oldState, movedByPiston);
        if (!level.isClientSide()) {
            ScreenClusterDiscovery.triggerRescanNear(level, pos);
        }
    }

    //? if >=26.1 {
    /*@Override
    protected void neighborChanged(BlockState state, Level level, BlockPos pos, Block neighborBlock,
                                   Orientation orientation, boolean movedByPiston) {
        super.neighborChanged(state, level, pos, neighborBlock, orientation, movedByPiston);
        handleNeighborChanged(level, pos);
    }*/
    //?} else {
    @Override
    protected void neighborChanged(BlockState state, Level level, BlockPos pos, Block neighborBlock,
                                   BlockPos neighborPos, boolean movedByPiston) {
        super.neighborChanged(state, level, pos, neighborBlock, neighborPos, movedByPiston);
        handleNeighborChanged(level, pos);
    }
    //?}

    private void handleNeighborChanged(Level level, BlockPos pos) {
        if (!level.isClientSide()) {
            ScreenClusterDiscovery.triggerRescanNear(level, pos);
        }
    }

    //? if >=26.1 {
    @Override
    protected void affectNeighborsAfterRemoval(BlockState state, ServerLevel level, BlockPos pos, boolean movedByPiston) {
        ScreenClusterDiscovery.triggerRescanNear(level, pos);
        super.affectNeighborsAfterRemoval(state, level, pos, movedByPiston);
    }
    //?} else {
    /*@Override
    protected void onRemove(BlockState state, Level level, BlockPos pos, BlockState newState, boolean movedByPiston) {
        if (level instanceof ServerLevel sl) ScreenClusterDiscovery.triggerRescanNear(sl, pos);
        super.onRemove(state, level, pos, newState, movedByPiston);
    }*/
    //?}
}
