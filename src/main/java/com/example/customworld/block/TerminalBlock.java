package com.example.customworld.block;

import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.world.InteractionHand;
import net.minecraft.world.InteractionResult;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.context.BlockPlaceContext;
import net.minecraft.world.level.BlockGetter;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.BaseEntityBlock;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.HorizontalDirectionalBlock;
import net.minecraft.world.level.block.RenderShape;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.block.state.BlockBehaviour;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.block.state.StateDefinition;
import net.minecraft.world.level.block.state.properties.DirectionProperty;
import net.minecraft.world.phys.BlockHitResult;
import net.minecraftforge.network.NetworkHooks;

import javax.annotation.Nullable;

/**
 * Terminal Block - A computer terminal that opens a custom UI.
 * Faces the player when placed.
 */
public class TerminalBlock extends BaseEntityBlock {

    public static final DirectionProperty FACING = HorizontalDirectionalBlock.FACING;

    public TerminalBlock(BlockBehaviour.Properties properties) {
        super(properties);
        this.registerDefaultState(this.stateDefinition.any().setValue(FACING, Direction.NORTH));
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
        return new TerminalBlockEntity(pos, state);
    }

    @Override
    public RenderShape getRenderShape(BlockState state) {
        return RenderShape.MODEL;
    }

    /**
     * Preserves the computer ID when the block is picked in creative mode.
     */
    @Override
    public ItemStack getCloneItemStack(BlockGetter level, BlockPos pos, BlockState state) {
        ItemStack stack = super.getCloneItemStack(level, pos, state);
        if (level.getBlockEntity(pos) instanceof TerminalBlockEntity te) {
            saveComputerIdToStack(stack, te);
        }
        return stack;
    }

    /**
     * Returns the drops for this block, preserving the computer ID.
     */
    @Override
    public java.util.List<ItemStack> getDrops(BlockState state, net.minecraft.world.level.storage.loot.LootParams.Builder builder) {
        // Get the block entity from the loot context
        BlockEntity blockEntity = builder.getOptionalParameter(net.minecraft.world.level.storage.loot.parameters.LootContextParams.BLOCK_ENTITY);
        if (blockEntity instanceof TerminalBlockEntity te) {
            ItemStack stack = new ItemStack(this);
            saveComputerIdToStack(stack, te);
            return java.util.List.of(stack);
        }
        return super.getDrops(state, builder);
    }

    /**
     * Saves the computer ID from a terminal block entity to an item stack.
     */
    private void saveComputerIdToStack(ItemStack stack, TerminalBlockEntity te) {
        CompoundTag tag = stack.getOrCreateTagElement("BlockEntityTag");
        tag.putUUID("computerId", te.getComputerId());
    }

    /**
     * Called after the block is placed. Restores the computer ID from the item if present.
     */
    @Override
    public void setPlacedBy(Level level, BlockPos pos, BlockState state, @Nullable net.minecraft.world.entity.LivingEntity placer, ItemStack stack) {
        super.setPlacedBy(level, pos, state, placer, stack);
        if (level.getBlockEntity(pos) instanceof TerminalBlockEntity te) {
            CompoundTag tag = stack.getTagElement("BlockEntityTag");
            if (tag != null && tag.hasUUID("computerId")) {
                te.setComputerId(tag.getUUID("computerId"));
            }
        }
    }

    @Override
    public InteractionResult use(BlockState state, Level level,
            BlockPos pos, Player player, InteractionHand hand, BlockHitResult hit) {
        if (!level.isClientSide && level.getBlockEntity(pos) instanceof TerminalBlockEntity te) {
            // Initialize WASM when terminal is first opened
            te.initializeWasm();
            NetworkHooks.openScreen((net.minecraft.server.level.ServerPlayer) player, te, pos);
        }
        return InteractionResult.sidedSuccess(level.isClientSide);
    }

    // ==================== Peripheral Discovery ====================

    /**
     * Called when a neighboring block changes.
     * Triggers peripheral rescan in the terminal.
     */
    @Override
    public void neighborChanged(BlockState state, Level level, BlockPos pos, Block neighborBlock, BlockPos neighborPos, boolean movedByPiston) {
        super.neighborChanged(state, level, pos, neighborBlock, neighborPos, movedByPiston);
        if (!level.isClientSide && level.getBlockEntity(pos) instanceof TerminalBlockEntity te) {
            te.onNeighborChanged();
        }
    }

    // ==================== Redstone Signal Output ====================

    /**
     * Indicates that this block can provide redstone power.
     */
    @Override
    public boolean isSignalSource(BlockState state) {
        return true;
    }

    /**
     * Returns the redstone power level for a given side.
     * @param direction The direction the signal is being queried FROM (opposite of output side)
     */
    @Override
    public int getSignal(BlockState state, BlockGetter level, BlockPos pos, Direction direction) {
        if (level.getBlockEntity(pos) instanceof TerminalBlockEntity te) {
            // direction is the side being queried FROM, so we use the opposite to get the output side
            return te.getRedstoneOutput(direction.getOpposite().ordinal());
        }
        return 0;
    }

    /**
     * Returns the direct redstone power level (for strong power through blocks).
     */
    @Override
    public int getDirectSignal(BlockState state, BlockGetter level, BlockPos pos, Direction direction) {
        return getSignal(state, level, pos, direction);
    }
}
