package com.example.evanscomputermod.block;

import com.example.evanscomputermod.computer.CableNetworkManager;
import com.mojang.serialization.MapCodec;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.level.BlockGetter;
import net.minecraft.world.level.Level;
//? if >=26.1 {
import net.minecraft.world.level.redstone.Orientation;
//?}
import net.minecraft.world.level.block.BaseEntityBlock;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.RenderShape;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.block.entity.BlockEntityTicker;
import net.minecraft.world.level.block.entity.BlockEntityType;
import net.minecraft.world.level.block.state.BlockBehaviour;
import net.minecraft.world.level.block.state.BlockState;

import org.jetbrains.annotations.Nullable;

/**
 * Redstone Network Block — a Java-side network device that emits redstone
 * signals driven by packets received over the in-world ethernet network. Does
 * not run WASM; all parsing happens in Java via
 * {@link com.example.evanscomputermod.computer.netdev.NetDevice}.
 */
public class RedstoneNetBlock extends BaseEntityBlock {

    public static final MapCodec<RedstoneNetBlock> CODEC = simpleCodec(RedstoneNetBlock::new);

    public RedstoneNetBlock(BlockBehaviour.Properties properties) {
        super(properties);
    }

    @Override
    protected MapCodec<? extends BaseEntityBlock> codec() {
        return CODEC;
    }

    @Nullable
    @Override
    public BlockEntity newBlockEntity(BlockPos pos, BlockState state) {
        return new RedstoneNetBlockEntity(pos, state);
    }

    @Nullable
    @Override
    public <T extends BlockEntity> BlockEntityTicker<T> getTicker(Level level, BlockState state, BlockEntityType<T> type) {
        return RedstoneNetBlockEntity.createTicker(level);
    }

    @Override
    protected RenderShape getRenderShape(BlockState state) {
        return RenderShape.MODEL;
    }

    @Override
    protected void onPlace(BlockState state, Level level, BlockPos pos,
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
    protected void onRemove(BlockState state, Level level, BlockPos pos, BlockState newState, boolean movedByPiston) {
        CableNetworkManager mgr = CableNetworkManager.getInstance();
        if (mgr != null) mgr.invalidateCache();
        super.onRemove(state, level, pos, newState, movedByPiston);
    }*/
    //?}

    // ==================== Redstone Signal Output ====================

    @Override
    protected boolean isSignalSource(BlockState state) {
        return true;
    }

    @Override
    protected int getSignal(BlockState state, BlockGetter level, BlockPos pos, Direction direction) {
        if (level.getBlockEntity(pos) instanceof RedstoneNetBlockEntity be) {
            return be.getRedstoneOutput(direction.getOpposite().ordinal());
        }
        return 0;
    }

    @Override
    protected int getDirectSignal(BlockState state, BlockGetter level, BlockPos pos, Direction direction) {
        return getSignal(state, level, pos, direction);
    }

    // ==================== Neighbor changes ====================

    //? if >=26.1 {
    @Override
    protected void neighborChanged(BlockState state, Level level, BlockPos pos, Block neighborBlock, Orientation orientation, boolean movedByPiston) {
        super.neighborChanged(state, level, pos, neighborBlock, orientation, movedByPiston);
        handleNeighborChanged(level);
    }
    //?} else {
    /*@Override
    protected void neighborChanged(BlockState state, Level level, BlockPos pos, Block neighborBlock, BlockPos neighborPos, boolean movedByPiston) {
        super.neighborChanged(state, level, pos, neighborBlock, neighborPos, movedByPiston);
        handleNeighborChanged(level);
    }*/
    //?}

    private void handleNeighborChanged(Level level) {
        if (!level.isClientSide()) {
            CableNetworkManager mgr = CableNetworkManager.getInstance();
            if (mgr != null) mgr.invalidateCache();
        }
    }
}
