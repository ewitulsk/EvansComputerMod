package com.example.evanscomputermod.card.printer;

//? if <=1.21.1 {
import com.example.evanscomputermod.card.CardContent;
import com.mojang.serialization.MapCodec;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.world.InteractionHand;
import net.minecraft.world.InteractionResult;
import net.minecraft.world.ItemInteractionResult;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.context.BlockPlaceContext;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.BaseEntityBlock;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.RenderShape;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.block.entity.BlockEntityTicker;
import net.minecraft.world.level.block.entity.BlockEntityType;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.block.state.StateDefinition;
import net.minecraft.world.level.block.state.properties.BlockStateProperties;
import net.minecraft.world.level.block.state.properties.BooleanProperty;
import net.minecraft.world.level.block.state.properties.DirectionProperty;
import net.minecraft.world.phys.BlockHitResult;
import org.jetbrains.annotations.Nullable;

/**
 * Card Printer (docs/cards/CARDS.md): paints a card's faces, prints its fields and
 * encodes its stripe, consuming dye; a graphics device only. It is loaded only through its
 * inventory: right-click opens the screen (a card slot, the output slot, sixteen dye slots);
 * hoppers and pipes insert on the sides and top and extract below.
 * The computer next to it drives it as the {@code card_printer} peripheral.
 */
public class CardPrinterBlock extends BaseEntityBlock {
    public static final MapCodec<CardPrinterBlock> CODEC = simpleCodec(CardPrinterBlock::new);
    public static final DirectionProperty FACING = BlockStateProperties.HORIZONTAL_FACING;
    /** Printing (the head is moving). */
    public static final BooleanProperty ACTIVE = BlockStateProperties.LIT;
    /** The body and the tray on top (models/card_printer.bbmodel). */
    private static final net.minecraft.world.phys.shapes.VoxelShape SHAPE = net.minecraft.world.phys.shapes.Shapes.or(
            Block.box(0, 0, 0, 16, 10, 16), Block.box(1, 10, 1, 15, 12, 15));

    public CardPrinterBlock(Properties properties) {
        super(properties);
        registerDefaultState(stateDefinition.any().setValue(FACING, Direction.NORTH).setValue(ACTIVE, false));
    }

    @Override
    protected MapCodec<? extends BaseEntityBlock> codec() {
        return CODEC;
    }

    @Override
    protected BlockState rotate(BlockState state, net.minecraft.world.level.block.Rotation rotation) {
        return state.setValue(FACING, rotation.rotate(state.getValue(FACING)));
    }

    @Override
    protected BlockState mirror(BlockState state, net.minecraft.world.level.block.Mirror mirror) {
        return state.rotate(mirror.getRotation(state.getValue(FACING)));
    }

    @Override
    protected void createBlockStateDefinition(StateDefinition.Builder<Block, BlockState> builder) {
        builder.add(FACING, ACTIVE);
    }

    @Override
    public BlockState getStateForPlacement(BlockPlaceContext ctx) {
        return defaultBlockState().setValue(FACING, ctx.getHorizontalDirection().getOpposite());
    }

    @Override
    protected RenderShape getRenderShape(BlockState state) {
        return RenderShape.MODEL;
    }

    @Override
    protected net.minecraft.world.phys.shapes.VoxelShape getShape(BlockState state, net.minecraft.world.level.BlockGetter level, BlockPos pos,
                                                                  net.minecraft.world.phys.shapes.CollisionContext ctx) {
        return SHAPE;
    }

    @Override
    public @Nullable BlockEntity newBlockEntity(BlockPos pos, BlockState state) {
        return new CardPrinterBlockEntity(pos, state);
    }

    @Override
    public @Nullable <T extends BlockEntity> BlockEntityTicker<T> getTicker(Level level, BlockState state, BlockEntityType<T> type) {
        return level.isClientSide() ? null : createTickerHelper(type, CardContent.CARD_PRINTER_BE.get(), CardPrinterBlockEntity::serverTick);
    }

    /** Right-click opens the screen; cards and dye go in through its slots (or hoppers and pipes), never by hand. */
    @Override
    protected InteractionResult useWithoutItem(BlockState state, Level level, BlockPos pos, Player player, BlockHitResult hit) {
        if (!(level.getBlockEntity(pos) instanceof CardPrinterBlockEntity be)) return InteractionResult.PASS;
        if (!level.isClientSide()) player.openMenu(be);
        return InteractionResult.sidedSuccess(level.isClientSide());
    }

    @Override
    protected ItemInteractionResult useItemOn(ItemStack stack, BlockState state, Level level, BlockPos pos, Player player,
                                              InteractionHand hand, BlockHitResult hit) {
        return ItemInteractionResult.PASS_TO_DEFAULT_BLOCK_INTERACTION;
    }

    @Override
    protected void onRemove(BlockState state, Level level, BlockPos pos, BlockState newState, boolean moved) {
        if (!state.is(newState.getBlock()) && level.getBlockEntity(pos) instanceof CardPrinterBlockEntity be)
            for (ItemStack s : be.drops()) Block.popResource(level, pos, s);
        super.onRemove(state, level, pos, newState, moved);
    }

    @Override
    protected boolean hasAnalogOutputSignal(BlockState state) {
        return true;
    }

    @Override
    protected int getAnalogOutputSignal(BlockState state, Level level, BlockPos pos) {
        return level.getBlockEntity(pos) instanceof CardPrinterBlockEntity be ? be.comparator() : 0;
    }
}
//?}
