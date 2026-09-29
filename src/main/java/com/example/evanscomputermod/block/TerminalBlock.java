package com.example.evanscomputermod.block;

import com.example.evanscomputermod.api.module.ModuleSlotVisual;
import com.example.evanscomputermod.module.InstalledModules;
import com.example.evanscomputermod.module.ModDataComponents;
import com.example.evanscomputermod.module.ModuleBays;
import com.example.evanscomputermod.module.ModuleInteraction;
import com.mojang.serialization.MapCodec;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.core.component.DataComponents;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.world.InteractionHand;
import net.minecraft.world.InteractionResult;
//? if <=1.21.1 {
import net.minecraft.world.ItemInteractionResult;
//?}
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
import net.minecraft.world.level.block.Mirror;
import net.minecraft.world.level.block.RenderShape;
import net.minecraft.world.level.block.Rotation;
//? if >=26.1 {
import net.minecraft.world.level.redstone.Orientation;
//?}
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.block.entity.BlockEntityTicker;
import net.minecraft.world.level.block.entity.BlockEntityType;
import net.minecraft.world.level.block.state.BlockBehaviour;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.block.state.StateDefinition;
import net.minecraft.world.level.block.state.properties.BooleanProperty;
import net.minecraft.world.level.block.state.properties.EnumProperty;
import net.minecraft.world.phys.BlockHitResult;

import org.jetbrains.annotations.Nullable;

/**
 * Terminal Block - A computer terminal that opens a custom UI.
 * Faces the player when placed.
 *
 * <p>Module bays: {@link #LEFT_BAY} / {@link #RIGHT_BAY} show an opened bay
 * on that side, and each bay slot property shows what is installed there, so
 * the modules render as part of the block model (see
 * {@link com.example.evanscomputermod.module.ModuleBays}).
 */
public class TerminalBlock extends BaseEntityBlock
        //? if <=1.21.1 {
        implements dev.ryanhcode.sable.api.block.BlockSubLevelAssemblyListener
        //?}
{

    /**
     * Rotate the FACING property so that bulk-move transactions (sable
     * physics-body assembly, structure blocks, etc.) produce a correctly
     * oriented block on the destination side. Default {@code Block.rotate}
     * returns the state unchanged, which would leave FACING pinned to the
     * original world direction.
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
    public void beforeMove(net.minecraft.server.level.ServerLevel from,
                           net.minecraft.server.level.ServerLevel to,
                           BlockState state, BlockPos oldPos, BlockPos newPos) {
        com.example.evanscomputermod.sable.SableAssemblyHooks.onTerminalBeforeMove(from, to, state, oldPos, newPos);
    }

    @Override
    public void afterMove(net.minecraft.server.level.ServerLevel from,
                          net.minecraft.server.level.ServerLevel to,
                          BlockState state, BlockPos oldPos, BlockPos newPos) {
        com.example.evanscomputermod.sable.SableAssemblyHooks.onTerminalAfterMove(from, to, state, oldPos, newPos);
    }
    //?}

    public static final EnumProperty<Direction> FACING = HorizontalDirectionalBlock.FACING;
    public static final BooleanProperty LEFT_BAY = BooleanProperty.create("left_bay");
    public static final BooleanProperty RIGHT_BAY = BooleanProperty.create("right_bay");
    /** One property per bay slot, indexed like {@link ModuleBays#SLOT_NAMES}. */
    @SuppressWarnings("unchecked")
    public static final EnumProperty<ModuleSlotVisual>[] SLOTS = new EnumProperty[ModuleBays.SLOTS];
    static {
        for (int i = 0; i < ModuleBays.SLOTS; i++) {
            SLOTS[i] = EnumProperty.create(ModuleBays.SLOT_NAMES[i], ModuleSlotVisual.class);
        }
    }
    public static final MapCodec<TerminalBlock> CODEC = simpleCodec(TerminalBlock::new);

    public TerminalBlock(BlockBehaviour.Properties properties) {
        super(properties);
        BlockState state = this.stateDefinition.any()
                .setValue(FACING, Direction.NORTH)
                .setValue(LEFT_BAY, false)
                .setValue(RIGHT_BAY, false);
        for (EnumProperty<ModuleSlotVisual> slot : SLOTS) state = state.setValue(slot, ModuleSlotVisual.EMPTY);
        this.registerDefaultState(state);
    }

    /** {@code state} showing {@code bays}' cards and modules. */
    public static BlockState withModules(BlockState state, ModuleBays bays) {
        state = state.setValue(LEFT_BAY, bays.hasBay(ModuleBays.LEFT)).setValue(RIGHT_BAY, bays.hasBay(ModuleBays.RIGHT));
        for (int i = 0; i < ModuleBays.SLOTS; i++) state = state.setValue(SLOTS[i], bays.visual(i));
        return state;
    }
    
    @Override
    protected MapCodec<? extends BaseEntityBlock> codec() {
        return CODEC;
    }
    
    @Override
    protected void createBlockStateDefinition(StateDefinition.Builder<Block, BlockState> builder) {
        builder.add(FACING, LEFT_BAY, RIGHT_BAY);
        builder.add(SLOTS);
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
            // Upgrades stay with the computer: cards and modules (with their
            // settings) ride on the dropped item and come back when it's placed.
            InstalledModules modules = te.getModuleBays().snapshot();
            if (!modules.isEmpty()) stack.set(ModDataComponents.INSTALLED_MODULES.get(), modules);
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
            InstalledModules modules = stack.get(ModDataComponents.INSTALLED_MODULES.get());
            if (modules != null && !level.isClientSide()) {
                te.restoreModules(modules);
            }
        }
    }
    
    // ==================== Module bays ====================

    //? if >=26.1 {
    @Override
    protected InteractionResult useItemOn(ItemStack stack, BlockState state, Level level, BlockPos pos,
                                          Player player, InteractionHand hand, BlockHitResult hit) {
        if (!ModuleInteraction.isBayItem(stack)) return InteractionResult.TRY_WITH_EMPTY_HAND;
        if (!level.isClientSide()) ModuleInteraction.useItem(stack, state, level, pos, player, hand, hit);
        return InteractionResult.SUCCESS;
    }
    //?} else {
    /*@Override
    protected ItemInteractionResult useItemOn(ItemStack stack, BlockState state, Level level, BlockPos pos,
                                              Player player, InteractionHand hand, BlockHitResult hit) {
        if (!ModuleInteraction.isBayItem(stack)) return ItemInteractionResult.PASS_TO_DEFAULT_BLOCK_INTERACTION;
        if (!level.isClientSide()) ModuleInteraction.useItem(stack, state, level, pos, player, hand, hit);
        return ItemInteractionResult.sidedSuccess(level.isClientSide());
    }*/
    //?}

    @Override
    protected InteractionResult useWithoutItem(BlockState state, Level level,
            BlockPos pos, Player player, BlockHitResult hit) {
        // Sneak + empty hand on a bay slot takes the module (or an empty bay's card) out.
        if (player.isSecondaryUseActive() && ModuleInteraction.aimedSlot(state, pos, hit) >= 0) {
            if (level.isClientSide()) return InteractionResult.SUCCESS;
            if (ModuleInteraction.sneakUse(state, level, pos, player, hit) == ModuleInteraction.Result.HANDLED) {
                return InteractionResult.SUCCESS;
            }
        }
        if (!level.isClientSide() && level.getBlockEntity(pos) instanceof TerminalBlockEntity te) {
            // Initialize WASM when terminal is first opened
            te.initializeWasm();
            player.openMenu(te, pos);
        }
        return InteractionResult.SUCCESS;
    }
    
    // ==================== Neighbor Updates ====================

    /**
     * Called when a neighboring block changes.
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
