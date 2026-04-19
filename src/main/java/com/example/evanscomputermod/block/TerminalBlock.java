package com.example.evanscomputermod.block;

import com.mojang.serialization.MapCodec;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.core.component.DataComponents;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.world.InteractionResult;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.component.CustomData;
import net.minecraft.world.item.context.BlockPlaceContext;
import net.minecraft.world.level.BlockGetter;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.LevelReader;
import net.minecraft.world.level.block.BaseEntityBlock;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.HorizontalDirectionalBlock;
import net.minecraft.world.level.block.RenderShape;
//? if >=26.1 {
import net.minecraft.world.level.redstone.Orientation;
//?}
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.block.entity.BlockEntityTicker;
import net.minecraft.world.level.block.entity.BlockEntityType;
import net.minecraft.world.level.block.state.BlockBehaviour;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.block.state.StateDefinition;
import net.minecraft.world.level.block.state.properties.EnumProperty;
import net.minecraft.world.phys.BlockHitResult;

import org.jetbrains.annotations.Nullable;

/**
 * Terminal Block - A computer terminal that opens a custom UI.
 * Faces the player when placed.
 */
public class TerminalBlock extends BaseEntityBlock {
    
    public static final EnumProperty<Direction> FACING = HorizontalDirectionalBlock.FACING;
    public static final MapCodec<TerminalBlock> CODEC = simpleCodec(TerminalBlock::new);
    
    public TerminalBlock(BlockBehaviour.Properties properties) {
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
        return new TerminalBlockEntity(pos, state);
    }

    @Nullable
    @Override
    public <T extends BlockEntity> BlockEntityTicker<T> getTicker(Level level, BlockState state, BlockEntityType<T> type) {
        return TerminalBlockEntity.createTicker(level);
    }

    @Override
    protected RenderShape getRenderShape(BlockState state) {
        return RenderShape.MODEL;
    }
    
    /**
     * Preserves the computer ID when the block is picked in creative mode.
     */
    //? if >=26.1 {
    @Override
    protected ItemStack getCloneItemStack(LevelReader level, BlockPos pos, BlockState state, boolean includeData) {
        ItemStack stack = super.getCloneItemStack(level, pos, state, includeData);
        if (level.getBlockEntity(pos) instanceof TerminalBlockEntity te) {
            saveComputerIdToStack(stack, te);
        }
        return stack;
    }
    //?} else {
    /*@Override
    public ItemStack getCloneItemStack(LevelReader level, BlockPos pos, BlockState state) {
        ItemStack stack = super.getCloneItemStack(level, pos, state);
        if (level.getBlockEntity(pos) instanceof TerminalBlockEntity te) {
            saveComputerIdToStack(stack, te);
        }
        return stack;
    }*/
    //?}
    
    /**
     * Returns the drops for this block, preserving the computer ID.
     */
    @Override
    protected java.util.List<ItemStack> getDrops(BlockState state, net.minecraft.world.level.storage.loot.LootParams.Builder builder) {
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
        CompoundTag tag = new CompoundTag();
        tag.putLong("computerIdMost", te.getComputerId().getMostSignificantBits());
        tag.putLong("computerIdLeast", te.getComputerId().getLeastSignificantBits());
        stack.set(DataComponents.CUSTOM_DATA, CustomData.of(tag));
    }
    
    /**
     * Called after the block is placed. Restores the computer ID from the item if present.
     */
    @Override
    public void setPlacedBy(Level level, BlockPos pos, BlockState state, net.minecraft.world.entity.LivingEntity placer, ItemStack stack) {
        super.setPlacedBy(level, pos, state, placer, stack);
        if (level.getBlockEntity(pos) instanceof TerminalBlockEntity te) {
            CustomData customData = stack.get(DataComponents.CUSTOM_DATA);
            if (customData != null) {
                CompoundTag tag = customData.copyTag();
                if (tag.contains("computerIdMost")) {
                    //? if >=26.1 {
                    java.util.UUID id = new java.util.UUID(tag.getLongOr("computerIdMost", 0L), tag.getLongOr("computerIdLeast", 0L));
                    //?} else
                    /*java.util.UUID id = new java.util.UUID(tag.getLong("computerIdMost"), tag.getLong("computerIdLeast"));*/
                    te.setComputerId(id);
                }
            }
        }
    }
    
    @Override
    protected InteractionResult useWithoutItem(BlockState state, Level level, 
            BlockPos pos, Player player, BlockHitResult hit) {
        if (!level.isClientSide() && level.getBlockEntity(pos) instanceof TerminalBlockEntity te) {
            // Initialize WASM when terminal is first opened
            te.initializeWasm();
            player.openMenu(te, pos);
        }
        return InteractionResult.SUCCESS;
    }
    
    // ==================== Peripheral Discovery ====================
    
    /**
     * Called when a neighboring block changes.
     * Triggers peripheral rescan in the terminal.
     */
    //? if >=26.1 {
    @Override
    protected void neighborChanged(BlockState state, Level level, BlockPos pos, Block neighborBlock, Orientation orientation, boolean movedByPiston) {
        super.neighborChanged(state, level, pos, neighborBlock, orientation, movedByPiston);
        handleNeighborChanged(level, pos);
    }
    //?} else {
    /*@Override
    protected void neighborChanged(BlockState state, Level level, BlockPos pos, Block neighborBlock, BlockPos neighborPos, boolean movedByPiston) {
        super.neighborChanged(state, level, pos, neighborBlock, neighborPos, movedByPiston);
        handleNeighborChanged(level, pos);
    }*/
    //?}

    private void handleNeighborChanged(Level level, BlockPos pos) {
        if (!level.isClientSide()) {
            if (level.getBlockEntity(pos) instanceof TerminalBlockEntity te) {
                te.onNeighborChanged();
            }
            // Invalidate cable network cache when neighbors change (cable placed/broken next to terminal)
            com.example.evanscomputermod.computer.CableNetworkManager cableMgr =
                    com.example.evanscomputermod.computer.CableNetworkManager.getInstance();
            if (cableMgr != null) {
                cableMgr.invalidateCache();
            }
        }
    }
    
    // ==================== Redstone Signal Output ====================
    
    /**
     * Indicates that this block can provide redstone power.
     */
    @Override
    protected boolean isSignalSource(BlockState state) {
        return true;
    }
    
    /**
     * Returns the redstone power level for a given side.
     * @param direction The direction the signal is being queried FROM (opposite of output side)
     */
    @Override
    protected int getSignal(BlockState state, BlockGetter level, BlockPos pos, Direction direction) {
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
    protected int getDirectSignal(BlockState state, BlockGetter level, BlockPos pos, Direction direction) {
        return getSignal(state, level, pos, direction);
    }
}
